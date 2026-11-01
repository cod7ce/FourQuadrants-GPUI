#import <AVFAudio/AVFAudio.h>
#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>
#import <mach/mach_time.h>

static dispatch_queue_t FQCompletionSoundQueue;
static AVAudioEngine *FQCompletionEngine;
static AVAudioPlayerNode *FQCompletionPlayer;
static AVAudioPCMBuffer *FQCompletionBuffer;
static id FQCompletionConfigurationObserver;
static dispatch_source_t FQCompletionHealthTimer;
static BOOL FQCompletionSoundEnabled;
static BOOL FQCompletionNeedsRebuild;
static uint64_t FQCompletionStartedAt;
static NSUInteger FQCompletionRequest;
static NSUInteger FQCompletionAttempt;
static BOOL FQCompletionPending;
static BOOL FQCompletionRetried;

static const NSTimeInterval FQCompletionHealthInterval = 30;
static const NSTimeInterval FQCompletionStallGrace = 1;
static const NSTimeInterval FQCompletionPlaybackTimeout = 1.5;

#if defined(__MAC_27_0)
// Avoid @available's compiler-runtime symbol in Rust's release link.
static BOOL FQCompletionUsesMacOS27API(void) {
    return NSProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 27;
}
#endif

static OSStatus FQReadSoundBytes(void *context, SInt64 position, UInt32 count, void *destination, UInt32 *actual) {
    NSData *data = (__bridge NSData *)context;
    *actual = 0;
    if (position < 0) return kAudioFilePositionError;
    if ((UInt64)position >= data.length) return noErr;
    *actual = (UInt32)MIN((NSUInteger)count, data.length - (NSUInteger)position);
    memcpy(destination, (const unsigned char *)data.bytes + position, *actual);
    return noErr;
}

static SInt64 FQSoundByteCount(void *context) {
    return (SInt64)((__bridge NSData *)context).length;
}

static AVAudioPCMBuffer *FQDecodeCompletionSound(NSData *data) {
    AudioFileID file = NULL;
    ExtAudioFileRef decoder = NULL;
    AVAudioPCMBuffer *buffer = nil;
    OSStatus status = AudioFileOpenWithCallbacks((__bridge void *)data, FQReadSoundBytes, NULL,
                                                FQSoundByteCount, NULL, kAudioFileAIFFType, &file);
    if (status == noErr) status = ExtAudioFileWrapAudioFileID(file, false, &decoder);
    SInt64 frameCount = 0;
    UInt32 size = sizeof(frameCount);
    if (status == noErr) status = ExtAudioFileGetProperty(decoder, kExtAudioFileProperty_FileLengthFrames, &size, &frameCount);
    // The embedded sound is 200 ms, mono, 48 kHz. Bound allocations if the asset is replaced incorrectly.
    if (status == noErr && frameCount > 0 && frameCount <= 48000) {
        AVAudioFormat *format = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:48000 channels:1];
        status = ExtAudioFileSetProperty(decoder, kExtAudioFileProperty_ClientDataFormat,
                                        sizeof(AudioStreamBasicDescription), format.streamDescription);
        if (status == noErr) {
            buffer = [[AVAudioPCMBuffer alloc] initWithPCMFormat:format frameCapacity:(AVAudioFrameCount)frameCount];
            buffer.frameLength = buffer.frameCapacity;
            UInt32 frames = buffer.frameCapacity;
            status = ExtAudioFileRead(decoder, &frames, buffer.mutableAudioBufferList);
            if (status == noErr && frames > 0) buffer.frameLength = frames;
            else buffer = nil;
        }
    }
    if (decoder) ExtAudioFileDispose(decoder);
    if (file) AudioFileClose(file);
    return buffer;
}

static BOOL FQEnsureCompletionAudio(void);
static void FQPlayCompletionAttempt(NSUInteger request);
static void FQRetryCompletionSound(NSUInteger request, NSUInteger attempt);

static void FQLogCompletionAudioException(NSString *activity, NSException *exception) {
    NSLog(@"Task completion audio %@ failed: %@: %@", activity,
          exception.name, exception.reason);
}

// All audio state and recovery work belongs to FQCompletionSoundQueue.
static void FQRetireCompletionAudio(void) {
    // stop also invokes buffer callbacks; invalidate them before touching the nodes.
    FQCompletionAttempt++;
    if (FQCompletionConfigurationObserver) {
        [NSNotificationCenter.defaultCenter removeObserver:FQCompletionConfigurationObserver];
        FQCompletionConfigurationObserver = nil;
    }
    // AVFAudio can raise from teardown too; cleanup must never escape its queue.
    @try {
        [FQCompletionPlayer stop];
    } @catch (NSException *exception) {
        FQLogCompletionAudioException(@"stop", exception);
    }
    @try {
        [FQCompletionEngine stop];
    } @catch (NSException *exception) {
        FQLogCompletionAudioException(@"stop", exception);
    }
    FQCompletionPlayer = nil;
    FQCompletionEngine = nil;
    FQCompletionStartedAt = 0;
}

static void FQCompletionConfigurationChanged(AVAudioEngine *engine) {
    if (engine != FQCompletionEngine) return;
    FQCompletionNeedsRebuild = YES;
    // Never restart an idle graph from a device notification. After wake,
    // starting its player can raise "player did not see an IO cycle"; the next
    // play request rebuilds the graph instead.
    if (!FQCompletionSoundEnabled || !FQCompletionPending) return;
    FQRetryCompletionSound(FQCompletionRequest, FQCompletionAttempt);
}

static BOOL FQCreateCompletionAudio(void) {
    FQRetireCompletionAudio();
    FQCompletionNeedsRebuild = YES;
    if (!FQCompletionBuffer) return NO;
    @try {
        FQCompletionEngine = [[AVAudioEngine alloc] init];
        FQCompletionPlayer = [[AVAudioPlayerNode alloc] init];
        FQCompletionPlayer.volume = 0.5f;
        [FQCompletionEngine attachNode:FQCompletionPlayer];
#if defined(__MAC_27_0)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wunguarded-availability-new"
        if (FQCompletionUsesMacOS27API()) {
            NSError *error = nil;
            if (![FQCompletionEngine connect:FQCompletionPlayer to:FQCompletionEngine.mainMixerNode
                                      format:FQCompletionBuffer.format error:&error]) {
                NSLog(@"Unable to connect task completion audio: %@", error.localizedDescription);
                FQRetireCompletionAudio();
                return NO;
            }
        } else
#pragma clang diagnostic pop
#endif
        {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
            [FQCompletionEngine connect:FQCompletionPlayer to:FQCompletionEngine.mainMixerNode
                                format:FQCompletionBuffer.format];
#pragma clang diagnostic pop
        }
        FQCompletionConfigurationObserver = [NSNotificationCenter.defaultCenter
            addObserverForName:AVAudioEngineConfigurationChangeNotification object:FQCompletionEngine queue:nil
            usingBlock:^(NSNotification *notification) {
                dispatch_async(FQCompletionSoundQueue, ^{
                    @autoreleasepool {
                        @try {
                            FQCompletionConfigurationChanged(notification.object);
                        } @catch (NSException *exception) {
                            FQLogCompletionAudioException(@"configuration change", exception);
                            FQCompletionNeedsRebuild = YES;
                        }
                    }
                });
            }];
        [FQCompletionEngine prepare];
    } @catch (NSException *exception) {
        FQLogCompletionAudioException(@"create", exception);
        FQRetireCompletionAudio();
        return NO;
    }
    FQCompletionNeedsRebuild = NO;
    return YES;
}

static BOOL FQStartCompletionEngine(void) {
    @try {
        if (!FQCompletionEngine.isRunning) {
            NSError *error = nil;
            if (![FQCompletionEngine startAndReturnError:&error]) {
                NSLog(@"Unable to start task completion audio: %@", error.localizedDescription);
                FQCompletionNeedsRebuild = YES;
                return NO;
            }
            FQCompletionStartedAt = mach_absolute_time();
        }
        if (!FQCompletionPlayer.isPlaying) {
#if defined(__MAC_27_0)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wunguarded-availability-new"
            if (FQCompletionUsesMacOS27API()) {
                NSError *error = nil;
                if (![FQCompletionPlayer playAndReturnError:&error]) {
                    NSLog(@"Unable to start task completion player: %@", error.localizedDescription);
                    FQCompletionNeedsRebuild = YES;
                    return NO;
                }
            } else
#pragma clang diagnostic pop
#endif
            {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
                [FQCompletionPlayer play]; // Compatibility with macOS before 27.
#pragma clang diagnostic pop
            }
        }
        return YES;
    } @catch (NSException *exception) {
        // playAndReturnError: can raise instead of returning an NSError on
        // macOS 27 (for example, "player did not see an IO cycle" after wake).
        FQLogCompletionAudioException(@"start", exception);
        FQCompletionNeedsRebuild = YES;
        return NO;
    }
}

static BOOL FQCompletionRenderIsFresh(AVAudioTime *render, uint64_t now) {
    // Host time is monotonic. Missing render timestamps are allowed only during startup.
    uint64_t last = render.isHostTimeValid ? render.hostTime : FQCompletionStartedAt;
    return last != 0 && (last >= now ||
        [AVAudioTime secondsForHostTime:now - last] < FQCompletionStallGrace);
}

static BOOL FQCompletionAudioIsHealthy(void) {
    uint64_t now = mach_absolute_time();
    return FQCompletionEngine.isRunning && FQCompletionPlayer.isPlaying &&
        FQCompletionRenderIsFresh(FQCompletionEngine.outputNode.lastRenderTime, now) &&
        FQCompletionRenderIsFresh(FQCompletionPlayer.lastRenderTime, now);
}

static BOOL FQEnsureCompletionAudio(void) {
    if (!FQCompletionSoundEnabled || !FQCompletionBuffer) return NO;
    if (FQCompletionEngine && FQCompletionPlayer &&
        (!FQCompletionEngine.isRunning || !FQCompletionPlayer.isPlaying ||
         !FQCompletionAudioIsHealthy())) {
        NSLog(@"Task completion audio stopped rendering; rebuilding");
        // Never restart a stale graph in place. After a device change or wake,
        // starting a player attached to the old graph can raise AVFAudio
        // exceptions before the new IO binding settles.
        FQCompletionNeedsRebuild = YES;
    }
    if (FQCompletionNeedsRebuild || !FQCompletionEngine || !FQCompletionPlayer) {
        if (!FQCreateCompletionAudio()) return NO;
    }
    return FQStartCompletionEngine();
}

static void FQCheckCompletionAudioHealth(void) {
    if (!FQCompletionSoundEnabled) return;
    // Idle audio is never started or restarted in the background. The health
    // timer only guards a sound that is already pending; the next play request
    // validates and rebuilds the graph on demand.
    if (!FQCompletionPending) return;
    if (FQCompletionNeedsRebuild || !FQCompletionAudioIsHealthy()) {
        FQRetryCompletionSound(FQCompletionRequest, FQCompletionAttempt);
    }
}

static void FQStartCompletionHealthTimer(void) {
    if (FQCompletionHealthTimer) return;
    FQCompletionHealthTimer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, FQCompletionSoundQueue);
    dispatch_source_set_timer(FQCompletionHealthTimer,
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(FQCompletionHealthInterval * NSEC_PER_SEC)),
        (uint64_t)(FQCompletionHealthInterval * NSEC_PER_SEC), (uint64_t)(2 * NSEC_PER_SEC));
    dispatch_source_set_event_handler(FQCompletionHealthTimer, ^{
        @autoreleasepool {
            @try {
                FQCheckCompletionAudioHealth();
            } @catch (NSException *exception) {
                FQLogCompletionAudioException(@"health check", exception);
                FQCompletionNeedsRebuild = YES;
            }
        }
    });
    dispatch_resume(FQCompletionHealthTimer);
}

static BOOL FQCompletionAttemptIsCurrent(NSUInteger request, NSUInteger attempt) {
    return FQCompletionSoundEnabled && FQCompletionPending &&
        request == FQCompletionRequest && attempt == FQCompletionAttempt;
}

static void FQRetryCompletionSound(NSUInteger request, NSUInteger attempt) {
    if (!FQCompletionAttemptIsCurrent(request, attempt)) return;
    FQCompletionNeedsRebuild = YES;
    if (FQCompletionRetried) {
        NSLog(@"Task completion audio recovery failed; abandoning this sound");
        FQCompletionPending = NO;
        FQRetireCompletionAudio();
        return;
    }
    NSLog(@"Task completion audio interrupted or timed out; rebuilding and retrying once");
    FQCompletionRetried = YES;
    FQPlayCompletionAttempt(request);
}

static void FQPlayCompletionAttempt(NSUInteger request) {
    if (!FQCompletionSoundEnabled || !FQCompletionPending || request != FQCompletionRequest) return;
    if (!FQEnsureCompletionAudio()) {
        FQRetryCompletionSound(request, FQCompletionAttempt);
        return;
    }
    NSUInteger attempt = ++FQCompletionAttempt;
    @try {
        // Keep the prepared player running; restarting it on every click adds latency.
        [FQCompletionPlayer scheduleBuffer:FQCompletionBuffer atTime:nil
            options:AVAudioPlayerNodeBufferInterrupts
            completionCallbackType:AVAudioPlayerNodeCompletionDataPlayedBack
            completionHandler:^(AVAudioPlayerNodeCompletionCallbackType type) {
                dispatch_async(FQCompletionSoundQueue, ^{
                    @autoreleasepool {
                        @try {
                            if (!FQCompletionAttemptIsCurrent(request, attempt)) return;
                            if (type == AVAudioPlayerNodeCompletionDataPlayedBack &&
                                !FQCompletionNeedsRebuild && FQCompletionAudioIsHealthy()) {
                                FQCompletionPending = NO;
                            } else {
                                FQRetryCompletionSound(request, attempt);
                            }
                        } @catch (NSException *exception) {
                            FQLogCompletionAudioException(@"playback callback", exception);
                            FQCompletionNeedsRebuild = YES;
                            FQRetryCompletionSound(request, attempt);
                        }
                    }
                });
            }];
    } @catch (NSException *exception) {
        FQLogCompletionAudioException(@"schedule", exception);
        FQCompletionNeedsRebuild = YES;
        FQRetryCompletionSound(request, attempt);
        return;
    }
    NSTimeInterval duration = FQCompletionBuffer.frameLength / FQCompletionBuffer.format.sampleRate;
    NSTimeInterval timeout = MAX(FQCompletionPlaybackTimeout,
        duration + MAX(0.0, FQCompletionPlayer.outputPresentationLatency) + FQCompletionStallGrace);
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,
        (int64_t)(timeout * NSEC_PER_SEC)), FQCompletionSoundQueue, ^{
        @autoreleasepool {
            @try {
                FQRetryCompletionSound(request, attempt);
            } @catch (NSException *exception) {
                FQLogCompletionAudioException(@"watchdog", exception);
                FQCompletionPending = NO;
                FQRetireCompletionAudio();
            }
        }
    });
}

void fq_prepare_task_complete_sound(const unsigned char *bytes, size_t length) {
    if (![NSThread isMainThread] || !bytes || length == 0) return;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        FQCompletionSoundQueue = dispatch_queue_create("app.fourquadrants.completion-sound", DISPATCH_QUEUE_SERIAL);
        NSData *data = [NSData dataWithBytes:bytes length:length];
        dispatch_async(FQCompletionSoundQueue, ^{
            @autoreleasepool {
                FQCompletionBuffer = FQDecodeCompletionSound(data);
                if (!FQCompletionBuffer) NSLog(@"Unable to decode task completion sound");
            }
        });
    });
}

void fq_set_task_complete_sound_enabled(int enabled) {
    if (![NSThread isMainThread] || !FQCompletionSoundQueue) return;
    dispatch_async(FQCompletionSoundQueue, ^{
        @autoreleasepool {
            @try {
                FQCompletionSoundEnabled = enabled != 0;
                if (FQCompletionSoundEnabled) {
                    // Warm the graph only when the user enables the setting.
                    FQEnsureCompletionAudio();
                    FQStartCompletionHealthTimer();
                } else {
                    FQCompletionRequest++;
                    FQCompletionPending = NO;
                    if (FQCompletionHealthTimer) {
                        dispatch_source_cancel(FQCompletionHealthTimer);
                        FQCompletionHealthTimer = nil;
                    }
                    FQRetireCompletionAudio();
                }
            } @catch (NSException *exception) {
                FQLogCompletionAudioException(@"settings update", exception);
                FQCompletionNeedsRebuild = YES;
            }
        }
    });
}

void fq_play_task_complete_sound(void) {
    if (![NSThread isMainThread] || !FQCompletionSoundQueue) return;
    dispatch_async(FQCompletionSoundQueue, ^{
        @autoreleasepool {
            @try {
                if (!FQCompletionSoundEnabled || !FQCompletionBuffer) return;
                FQCompletionRequest++;
                FQCompletionPending = YES;
                FQCompletionRetried = NO;
                FQPlayCompletionAttempt(FQCompletionRequest);
            } @catch (NSException *exception) {
                FQLogCompletionAudioException(@"play request", exception);
                FQCompletionPending = NO;
                FQRetireCompletionAudio();
            }
        }
    });
}
