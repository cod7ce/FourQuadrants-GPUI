#import <AVFAudio/AVFAudio.h>
#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>

static dispatch_queue_t FQCompletionSoundQueue;
static AVAudioEngine *FQCompletionEngine;
static AVAudioPlayerNode *FQCompletionPlayer;
static AVAudioPCMBuffer *FQCompletionBuffer;
static id FQCompletionConfigurationObserver;

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

static BOOL FQStartCompletionEngine(void) {
    if (!FQCompletionEngine) return NO;
    if (!FQCompletionEngine.isRunning) {
        NSError *error = nil;
        if (![FQCompletionEngine startAndReturnError:&error]) {
            NSLog(@"Unable to start task completion audio: %@", error.localizedDescription);
            return NO;
        }
    }
    if (!FQCompletionPlayer.isPlaying) {
#if defined(__MAC_27_0)
        if (@available(macOS 27.0, *)) {
            NSError *error = nil;
            if (![FQCompletionPlayer playAndReturnError:&error]) {
                NSLog(@"Unable to start task completion player: %@", error.localizedDescription);
                return NO;
            }
        } else
#endif
        {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
            [FQCompletionPlayer play]; // Compatibility with macOS before 27.
#pragma clang diagnostic pop
        }
    }
    return YES;
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
                if (!FQCompletionBuffer) {
                    NSLog(@"Unable to decode task completion sound");
                    return;
                }
                FQCompletionEngine = [[AVAudioEngine alloc] init];
                FQCompletionPlayer = [[AVAudioPlayerNode alloc] init];
                FQCompletionPlayer.volume = 0.5f;
                [FQCompletionEngine attachNode:FQCompletionPlayer];
#if defined(__MAC_27_0)
                if (@available(macOS 27.0, *)) {
                    NSError *error = nil;
                    if (![FQCompletionEngine connect:FQCompletionPlayer to:FQCompletionEngine.mainMixerNode
                                              format:FQCompletionBuffer.format error:&error]) {
                        NSLog(@"Unable to connect task completion audio: %@", error.localizedDescription);
                        FQCompletionEngine = nil;
                        return;
                    }
                } else
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
                    usingBlock:^(NSNotification * __unused notification) {
                        // Device changes may stop the engine; restart it on its owning queue.
                        dispatch_async(FQCompletionSoundQueue, ^{ FQStartCompletionEngine(); });
                    }];
                [FQCompletionEngine prepare];
                // Keep the output path running silently between clicks, with no looping sound.
                FQStartCompletionEngine();
            }
        });
    });
}

void fq_play_task_complete_sound(void) {
    if (![NSThread isMainThread] || !FQCompletionSoundQueue) return;
    dispatch_async(FQCompletionSoundQueue, ^{
        @autoreleasepool {
            if (!FQCompletionBuffer || !FQStartCompletionEngine()) return;
            // Replace a still-playing burst without stopping the engine or stacking sounds.
            [FQCompletionPlayer scheduleBuffer:FQCompletionBuffer atTime:nil
                                       options:AVAudioPlayerNodeBufferInterrupts completionHandler:nil];
        }
    });
}
