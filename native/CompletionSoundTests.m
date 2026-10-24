// Exercise recovery and callback ordering without opening an audio device.
#import <AVFAudio/AVFAudio.h>
#import <Foundation/Foundation.h>
#import <mach/mach_time.h>
#include <assert.h>

static NSUInteger FQTestEngineCount;
static BOOL FQTestStartFails;

@interface FQFakeAudioNode : NSObject
@property (nonatomic) BOOL stalled;
@property (nonatomic) BOOL missing;
@property (nonatomic, readonly) AVAudioTime *lastRenderTime;
@end
@implementation FQFakeAudioNode
- (AVAudioTime *)lastRenderTime {
    if (self.missing) return nil;
    uint64_t now = mach_absolute_time();
    return [AVAudioTime timeWithHostTime:self.stalled ? now - [AVAudioTime hostTimeForSeconds:5] : now];
}
@end

@interface FQFakeAudioPlayer : FQFakeAudioNode
@property (nonatomic) float volume;
@property (nonatomic) NSTimeInterval outputPresentationLatency;
@property (nonatomic, getter=isPlaying) BOOL playing;
@property (nonatomic) NSUInteger schedules;
@property (nonatomic) AVAudioPlayerNodeCompletionCallbackType callbackType;
@property (nonatomic, strong) NSMutableArray *callbacks;
- (void)play;
- (BOOL)playAndReturnError:(NSError **)error;
- (void)stop;
- (void)finish;
- (void)scheduleBuffer:(AVAudioPCMBuffer *)buffer atTime:(AVAudioTime *)time
    options:(AVAudioPlayerNodeBufferOptions)options
    completionCallbackType:(AVAudioPlayerNodeCompletionCallbackType)type
    completionHandler:(AVAudioPlayerNodeCompletionHandler)handler;
@end
@implementation FQFakeAudioPlayer
- (instancetype)init {
    if ((self = [super init])) self.callbacks = [NSMutableArray array];
    return self;
}
- (void)play { self.playing = YES; }
- (BOOL)playAndReturnError:(NSError **)error { (void)error; [self play]; return YES; }
- (void)finish {
    NSArray *callbacks = self.callbacks;
    self.callbacks = [NSMutableArray array];
    for (AVAudioPlayerNodeCompletionHandler callback in callbacks) callback(self.callbackType);
}
- (void)stop { self.playing = NO; [self finish]; }
- (void)scheduleBuffer:(AVAudioPCMBuffer *)buffer atTime:(AVAudioTime *)time
    options:(AVAudioPlayerNodeBufferOptions)options
    completionCallbackType:(AVAudioPlayerNodeCompletionCallbackType)type
    completionHandler:(AVAudioPlayerNodeCompletionHandler)handler {
    assert(buffer.frameLength == 9600 && time == nil);
    assert(options == AVAudioPlayerNodeBufferInterrupts);
    [self finish];
    self.schedules++;
    self.callbackType = type;
    [self.callbacks addObject:[handler copy]];
}
@end

@interface FQFakeAudioEngine : NSObject
@property (nonatomic, getter=isRunning) BOOL running;
@property (nonatomic, strong) FQFakeAudioNode *outputNode;
@property (nonatomic, strong) FQFakeAudioNode *mainMixerNode;
@property (nonatomic, strong) FQFakeAudioPlayer *player;
- (void)attachNode:(FQFakeAudioPlayer *)player;
- (void)connect:(FQFakeAudioPlayer *)player to:(FQFakeAudioNode *)node format:(AVAudioFormat *)format;
- (BOOL)connect:(FQFakeAudioPlayer *)player to:(FQFakeAudioNode *)node format:(AVAudioFormat *)format error:(NSError **)error;
- (BOOL)startAndReturnError:(NSError **)error;
- (void)prepare;
- (void)stop;
@end
@implementation FQFakeAudioEngine
- (instancetype)init {
    if ((self = [super init])) {
        FQTestEngineCount++;
        self.outputNode = [[FQFakeAudioNode alloc] init];
        self.mainMixerNode = [[FQFakeAudioNode alloc] init];
    }
    return self;
}
- (void)attachNode:(FQFakeAudioPlayer *)player { self.player = player; }
- (void)connect:(FQFakeAudioPlayer *)player to:(FQFakeAudioNode *)node format:(AVAudioFormat *)format {
    (void)player; (void)node; (void)format;
}
- (BOOL)connect:(FQFakeAudioPlayer *)player to:(FQFakeAudioNode *)node format:(AVAudioFormat *)format error:(NSError **)error {
    (void)error;
    [self connect:player to:node format:format];
    return YES;
}
- (BOOL)startAndReturnError:(NSError **)error {
    (void)error;
    self.running = !FQTestStartFails;
    return self.running;
}
- (void)prepare {}
- (void)stop { self.running = NO; }
@end

#define AVAudioEngine FQFakeAudioEngine
#define AVAudioPlayerNode FQFakeAudioPlayer
#import "CompletionSound.m"
#undef AVAudioEngine
#undef AVAudioPlayerNode

static void FQDrainSoundQueue(void) { dispatch_sync(FQCompletionSoundQueue, ^{}); }
static void FQEnableSound(BOOL enabled) {
    fq_set_task_complete_sound_enabled(enabled);
    FQDrainSoundQueue();
}
static void FQRequestSound(void) {
    fq_play_task_complete_sound();
    FQDrainSoundQueue();
}
static void FQFinishSound(void) {
    dispatch_sync(FQCompletionSoundQueue, ^{ [FQCompletionPlayer finish]; });
    FQDrainSoundQueue();
    assert(!FQCompletionPending);
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        assert(argc == 2);
        NSData *data = [NSData dataWithContentsOfFile:@(argv[1])];
        assert(data != nil);
        fq_prepare_task_complete_sound(data.bytes, data.length);
        FQDrainSoundQueue();
        FQEnableSound(YES);
        assert(FQCompletionEngine.isRunning && FQCompletionPlayer.isPlaying);
        NSUInteger created = FQTestEngineCount;
        dispatch_sync(FQCompletionSoundQueue, ^{
            FQCheckCompletionAudioHealth();
            FQCheckCompletionAudioHealth();
            uint64_t now = mach_absolute_time();
            FQCompletionStartedAt = now;
            assert(FQCompletionRenderIsFresh(nil, now));
            assert(!FQCompletionRenderIsFresh(nil, now + [AVAudioTime hostTimeForSeconds:2]));
        });
        assert(FQTestEngineCount == created);
        puts("healthy idle path stays warm; missing render clock has bounded startup grace: passed");

        FQRequestSound();
        assert(FQCompletionPlayer.callbackType == AVAudioPlayerNodeCompletionDataPlayedBack);
        FQRequestSound(); // Interrupt invokes the old request's callback.
        FQDrainSoundQueue();
        assert(FQCompletionPending && !FQCompletionRetried && FQTestEngineCount == created);
        FQFinishSound();
        puts("played-back callback and overlapping request cancellation: passed");

        dispatch_sync(FQCompletionSoundQueue, ^{
            FQCompletionPlayer.stalled = YES;
            FQCheckCompletionAudioHealth();
        });
        assert(FQTestEngineCount == ++created);
        dispatch_sync(FQCompletionSoundQueue, ^{
            FQCompletionEngine.outputNode.stalled = YES;
            FQCheckCompletionAudioHealth();
        });
        assert(FQTestEngineCount == ++created);
        puts("player and output render stalls rebuild independently: passed");

        FQRequestSound();
        dispatch_sync(FQCompletionSoundQueue, ^{ [FQCompletionPlayer stop]; });
        FQDrainSoundQueue();
        assert(FQTestEngineCount == ++created && FQCompletionPending && FQCompletionRetried);
        FQFinishSound();
        puts("stop-triggered callback retries the same sound: passed");

        FQRequestSound();
        dispatch_sync(FQCompletionSoundQueue, ^{
            FQFakeAudioEngine *old = FQCompletionEngine;
            FQCompletionConfigurationChanged(old);
            assert(FQCompletionEngine != old);
            NSUInteger count = FQTestEngineCount;
            FQCompletionConfigurationChanged(old);
            assert(FQTestEngineCount == count);
        });
        FQDrainSoundQueue();
        assert(FQTestEngineCount == ++created && FQCompletionPending && FQCompletionRetried);
        FQFinishSound();
        puts("configuration change replays once; obsolete engine notification is ignored: passed");

        FQRequestSound();
        [NSThread sleepForTimeInterval:1.7]; // Exercise the actual delayed watchdog.
        FQDrainSoundQueue();
        assert(FQTestEngineCount == ++created && FQCompletionRetried && FQCompletionPending);
        FQFinishSound();
        puts("missing playback callback rebuilds and retries without a new request: passed");

        dispatch_sync(FQCompletionSoundQueue, ^{ FQCompletionPlayer.outputPresentationLatency = 2; });
        FQRequestSound();
        [NSThread sleepForTimeInterval:1.7];
        FQDrainSoundQueue();
        assert(FQTestEngineCount == created && FQCompletionPending && !FQCompletionRetried);
        FQFinishSound();
        dispatch_sync(FQCompletionSoundQueue, ^{ FQCompletionPlayer.outputPresentationLatency = 0; });
        puts("device presentation latency extends the playback deadline: passed");

        FQRequestSound();
        dispatch_sync(FQCompletionSoundQueue, ^{
            FQRetryCompletionSound(FQCompletionRequest, FQCompletionAttempt);
            FQRetryCompletionSound(FQCompletionRequest, FQCompletionAttempt);
        });
        FQDrainSoundQueue();
        assert(FQTestEngineCount == ++created && !FQCompletionPending && !FQCompletionEngine);
        puts("repeated failure abandons the sound after one retry: passed");

        FQEnableSound(YES);
        created++;
        FQRequestSound();
        FQEnableSound(NO);
        assert(!FQCompletionEngine && !FQCompletionHealthTimer && !FQCompletionPending);
        FQEnableSound(YES);
        created++;
        [NSThread sleepForTimeInterval:1.7];
        FQDrainSoundQueue();
        assert(FQTestEngineCount == created && !FQCompletionPending);
        puts("disable cancels callbacks and watchdogs across re-enable: passed");

        FQEnableSound(NO);
        FQTestStartFails = YES;
        FQEnableSound(YES);
        FQRequestSound();
        assert(!FQCompletionPending && !FQCompletionEngine);
        FQTestStartFails = NO;
        dispatch_sync(FQCompletionSoundQueue, ^{ FQCheckCompletionAudioHealth(); });
        assert(FQCompletionEngine.isRunning && FQCompletionPlayer.isPlaying);
        FQEnableSound(NO);
        puts("startup failure is bounded and a later health check can recover: passed");
    }
    return 0;
}
