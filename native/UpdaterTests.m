// Exercise the bridge without loading or executing Sparkle.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wnullability-completeness"
#import "Updater.m"
#pragma clang diagnostic pop
#include <assert.h>
#include <stdio.h>

@interface FQFakeUpdater : NSObject
@property (nonatomic) BOOL canCheckForUpdates;
@property (nonatomic) BOOL sessionInProgress;
@end
@implementation FQFakeUpdater
@end

@interface FQFakeItem : NSObject
@property (nonatomic, copy) NSString *displayVersionString;
@end
@implementation FQFakeItem
@end

@interface FQTestAppDelegate : NSObject <NSApplicationDelegate>
@end
@implementation FQTestAppDelegate
@end

@interface FQExistingAppDelegate : NSObject <NSApplicationDelegate>
@end
@implementation FQExistingAppDelegate
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)application {
    (void)application;
    return NSTerminateCancel;
}
@end

@interface FQFakeApplication : NSObject
@property (nonatomic, strong) id<NSApplicationDelegate> delegate;
@end
@implementation FQFakeApplication
@end

static BOOL gTestSaveAllowed;
static NSUInteger gTestSaves;

static bool FQTestSaveHandler(void) {
    gTestSaves++;
    char *json = fq_updater_status(); // Rust may call back into bridge status.
    free(json);
    assert([NSApp.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
    return gTestSaveAllowed;
}

static NSDictionary *FQTestStatus(void) {
    char *json = fq_updater_status();
    assert(json != NULL);
    NSData *data = [NSData dataWithBytes:json length:strlen(json)];
    free(json);
    NSDictionary *result = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    assert(result != nil);
    return result;
}

static void FQExpectState(NSString *value) {
    assert([FQTestStatus()[@"state"] isEqualToString:value]);
}

static void FQDrainMainQueue(void) {
    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
}

// Real AppKit termination in an isolated process with no windows and no Sparkle.
// Both requests originate on the GCD main queue, so a nested NSTerminateLater
// loop would deadlock. Synchronous save approval must cancel once, then exit.
static void FQTestAppKitTermination(void) {
    NSApplication *application = NSApplication.sharedApplication;
    [application setActivationPolicy:NSApplicationActivationPolicyProhibited];
    FQTestAppDelegate *delegate = [[FQTestAppDelegate alloc] init];
    application.delegate = delegate;
    fq_updater_set_termination_handler(FQTestSaveHandler);
    assert(FQInstallTerminationGuard(application));
    assert(FQTerminationGuardIsActive());
    gEnabled = YES;
    gInstallStarted = YES;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC), dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        fputs("AppKit termination test timed out\n", stderr);
        _Exit(1);
    });
    dispatch_async(dispatch_get_main_queue(), ^{
        [application terminate:nil];
        assert(gTestSaves == 1);
        FQExpectState(@"error");
        gTestSaveAllowed = YES;
        dispatch_async(dispatch_get_main_queue(), ^{
            puts("AppKit synchronous save cancellation and retry approval: passed");
            fflush(stdout);
            [application terminate:nil];
            abort();
        });
    });
    [application run];
    (void)delegate;
    abort();
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc > 1 && strcmp(argv[1], "appkit") == 0) {
            FQTestAppKitTermination();
            return 1;
        }
        if (argc > 1 && strcmp(argv[1], "disabled") == 0) {
            fq_updater_initialize(false);
            FQExpectState(@"disabled");
            fq_updater_initialize(true);
            fq_updater_check();
            fq_updater_resume_install(true);
            FQExpectState(@"disabled");
            assert(gUpdaterController == nil && gSparkleBundle == nil);
            assert(!gInstallStarted && ![FQTestStatus()[@"can_check"] boolValue]);
            puts("disabled process latch: passed (no framework or guard loaded)");
            return 0;
        }
        if (argc > 1 && strcmp(argv[1], "unavailable") == 0) {
            fq_updater_initialize(true);
            FQExpectState(@"unavailable");
            assert(gUpdaterController == nil && gSparkleBundle == nil);
            assert(![FQTestStatus()[@"can_check"] boolValue]);
            puts("non-app availability: passed (no framework loaded)");
            return 0;
        }

        FQFakeApplication *application = [[FQFakeApplication alloc] init];
        NSApp = (NSApplication *)application;
        application.delegate = [[FQExistingAppDelegate alloc] init];
        assert(!FQInstallTerminationGuard(NSApp));
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        application.delegate = [[FQTestAppDelegate alloc] init];
        assert(FQInstallTerminationGuard(NSApp));
        assert(FQInstallTerminationGuard(NSApp)); // Repeated initialization is safe.
        assert(FQTerminationGuardIsActive());
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateNow);
        fq_updater_set_termination_handler(FQTestSaveHandler);

        gEnabled = YES;
        FQFakeUpdater *fake = [[FQFakeUpdater alloc] init];
        SPUUpdater *updater = (SPUUpdater *)fake;
        gUpdater = updater;
        FQUpdaterDelegate *delegate = [[FQUpdaterDelegate alloc] init];
        if (argc > 1 && strcmp(argv[1], "disable-queued") == 0) {
            __block BOOL invoked = NO;
            assert([delegate updater:updater shouldPostponeRelaunchForUpdate:nil untilInvokingBlock:^{ invoked = YES; }]);
            fq_updater_resume_install(true);
            fq_updater_initialize(false);
            FQDrainMainQueue();
            assert(!invoked && !gInstallApproved);
            FQExpectState(@"disabled");
            puts("disable cancels queued install approval: passed");
            return 0;
        }
        FQFakeItem *fakeItem = [[FQFakeItem alloc] init];
        fakeItem.displayVersionString = @"0.9.7\"\n验证";
        SUAppcastItem *item = (SUAppcastItem *)fakeItem;
        NSError *error = nil;
        assert([delegate updater:updater mayPerformUpdateCheck:0 error:&error]);
        assert(![delegate updater:updater mayPerformUpdateCheck:1 error:&error]);
        assert(error != nil);
        assert(![delegate updaterShouldPromptForPermissionToCheckForUpdates:updater]);
        assert([delegate allowedSystemProfileKeysForUpdater:updater].count == 0);

        [delegate updater:updater didFindValidUpdate:item];
        FQExpectState(@"available");
        assert([FQTestStatus()[@"version"] isEqualToString:fakeItem.displayVersionString]);
        NSError *noUpdate = [NSError errorWithDomain:@"SyntheticSparkle" code:1001 userInfo:nil];
        [delegate updaterDidNotFindUpdate:updater error:noUpdate];
        [delegate updater:updater didAbortWithError:noUpdate];
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:noUpdate];
        FQExpectState(@"current");
        assert(FQTestStatus()[@"version"] == nil);
        NSError *failure = [NSError errorWithDomain:@"SyntheticNetwork" code:1001 userInfo:@{NSLocalizedDescriptionKey: @"Offline"}];
        [delegate updater:updater didAbortWithError:failure];
        FQExpectState(@"error");
        fake.canCheckForUpdates = YES;
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:failure];
        assert([FQTestStatus()[@"can_check"] boolValue]);

        assert([delegate updaterShouldRelaunchApplication:updater]);
        assert(gInstallStarted);
        __block NSUInteger installs = 0;
        assert([delegate updater:updater shouldPostponeRelaunchForUpdate:item untilInvokingBlock:^{
            assert([delegate updaterShouldRelaunchApplication:updater]);
            installs++;
        }]);
        FQExpectState(@"install_pending");
        assert(![FQTestStatus()[@"can_check"] boolValue]);
        fq_updater_resume_install(false);
        FQExpectState(@"error");
        assert(installs == 0 && gPendingInstall != nil);
        assert([FQTestStatus()[@"can_check"] boolValue]);
        fq_updater_check();
        FQExpectState(@"install_pending");
        fq_updater_resume_install(true);
        FQExpectState(@"checking");
        assert(installs == 0);
        fq_updater_resume_install(true);
        FQDrainMainQueue();
        assert(installs == 1 && gPendingInstall == nil && !gInstallApproved);

        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestSaves == 1);
        FQExpectState(@"error");
        gTestSaveAllowed = YES;
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateNow);
        assert(gTestSaves == 2);
        gTestSaveAllowed = NO; // A new edit after approval needs a fresh save.
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestSaves == 3);
        fq_updater_set_termination_handler(NULL);
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestSaves == 3);
        fq_updater_set_termination_handler(FQTestSaveHandler);
        [delegate updater:updater didAbortWithError:failure];
        [delegate updater:updater didFinishUpdateCycleForUpdateCheck:0 error:failure];
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestSaves == 4);

        assert(![delegate updaterShouldRelaunchApplication:updater]);
        FQExpectState(@"error");
        assert(gRelaunchBlocked && ![FQTestStatus()[@"can_check"] boolValue]);
        fq_updater_initialize(false);
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateCancel);
        assert(gTestSaves == 5); // Disabling cannot remove an armed draft guard.
        fq_updater_workspace_closed(); // Successfully saved/closed, entity gone.
        assert([application.delegate applicationShouldTerminate:NSApp] == NSTerminateNow);
        fq_updater_workspace_closed();
        assert(gTestSaves == 5);
        puts("synthetic statuses, retries, install approval, final termination and close guards: passed");
        NSApp = nil;
    }
    return 0;
}
