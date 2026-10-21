#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#import <objc/runtime.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

@class SPUUpdater;
@class SUAppcastItem;

// Sparkle 2.10's public Objective-C ABI, resolved only after loading the framework.
// These declarations introduce no Sparkle class or symbol references to the linker.
@interface SPUStandardUpdaterController : NSObject
- (instancetype)initWithStartingUpdater:(BOOL)startUpdater
                       updaterDelegate:(nullable id)updaterDelegate
                    userDriverDelegate:(nullable id)userDriverDelegate;
@property (nonatomic, readonly) SPUUpdater *updater;
- (void)checkForUpdates:(nullable id)sender;
@end

@interface SPUUpdater : NSObject
@property (nonatomic) BOOL automaticallyChecksForUpdates;
@property (nonatomic) BOOL automaticallyDownloadsUpdates;
@property (nonatomic) BOOL sendsSystemProfile;
@property (nonatomic, readonly) BOOL canCheckForUpdates;
@property (nonatomic, readonly) BOOL sessionInProgress;
- (BOOL)startUpdater:(NSError * __autoreleasing _Nullable *)error;
@end

@interface SUAppcastItem : NSObject
@property (nonatomic, readonly, nullable) NSString *displayVersionString;
@end

@class FQUpdaterDelegate;

static NSBundle *gSparkleBundle;
static SPUStandardUpdaterController *gUpdaterController;
static FQUpdaterDelegate *gUpdaterDelegate;
static SPUUpdater *gUpdater;
static NSString *gState = @"unavailable";
static NSString *gMessage = @"Updater has not been initialized.";
static NSString *gVersion;
// Sparkle objects and installation state are main-thread-only. The status snapshot
// uses this class lock so Rust may poll it from any thread without entering Sparkle.
static BOOL gEnabled;
static BOOL gPermanentlyDisabled;
static BOOL gCanCheck;
static BOOL gPostponementObserved;
static BOOL gInstallApproved;
static BOOL gResumeQueued;
static BOOL gRelaunchBlocked;
static BOOL gInstallStarted;
static BOOL gWorkspaceClosed;
static BOOL gTerminationCallbackActive;
static bool (*gTerminationHandler)(void);
static NSError *gNoUpdateError;
static void (^gPendingInstall)(void);

// Sparkle's installer sends a normal AppKit quit event, potentially long after
// save approval or via its own retry handler. Gate each actual termination, not
// merely the initial Sparkle continuation. Leave ordinary quits unchanged until
// this process has started an installation, then keep guarding for its lifetime.
static NSApplicationTerminateReply FQApplicationShouldTerminate(id delegate, SEL selector, NSApplication *application) {
    (void)delegate;
    (void)selector;
    (void)application;
    bool (*handler)(void);
    @synchronized ([NSObject class]) {
        if (!gInstallStarted || gWorkspaceClosed) return NSTerminateNow;
        if (!NSThread.isMainThread || gTerminationCallbackActive || gTerminationHandler == NULL) return NSTerminateCancel;
        handler = gTerminationHandler;
        gTerminationCallbackActive = YES;
    }
    // No native lock crosses this call into Rust. Rust must catch panics and
    // reentrant GPUI borrows, returning false when it cannot safely save drafts.
    bool allowed = handler();
    @synchronized ([NSObject class]) {
        gTerminationCallbackActive = NO;
        if (!allowed) {
            gState = @"error";
            gMessage = @"Termination was cancelled because draft saving did not complete. Retry after saving your changes.";
            gCanCheck = !gRelaunchBlocked && gEnabled && gUpdater.canCheckForUpdates;
        }
    }
    return allowed ? NSTerminateNow : NSTerminateCancel;
}

static BOOL FQInstallTerminationGuard(NSApplication *application) {
    id<NSApplicationDelegate> delegate = application.delegate;
    if (application == nil || delegate == nil) return NO;
    Class delegateClass = object_getClass(delegate);
    SEL selector = @selector(applicationShouldTerminate:);
    Method existingMethod = class_getInstanceMethod(delegateClass, selector);
    if (existingMethod != NULL) {
        return method_getImplementation(existingMethod) == (IMP)FQApplicationShouldTerminate;
    }
    // Do not replace a host delegate's implementation or forwarding behavior.
    if ([delegate respondsToSelector:selector]) return NO;
    struct objc_method_description signature = protocol_getMethodDescription(@protocol(NSApplicationDelegate), selector, NO, YES);
    if (signature.types == NULL || !class_addMethod(delegateClass, selector, (IMP)FQApplicationShouldTerminate, signature.types)) return NO;
    // AppKit can cache which optional delegate methods are implemented.
    application.delegate = nil;
    application.delegate = delegate;
    return YES;
}

static BOOL FQTerminationGuardIsActive(void) {
    id delegate = NSApp.delegate;
    Method method = delegate == nil ? NULL : class_getInstanceMethod(object_getClass(delegate), @selector(applicationShouldTerminate:));
    return method != NULL && method_getImplementation(method) == (IMP)FQApplicationShouldTerminate;
}

static void FQSetStatus(NSString *state, NSString *message, NSString * _Nullable version, BOOL canCheck) {
    @synchronized ([NSObject class]) {
        gState = [state copy];
        gMessage = [message copy];
        gVersion = [version copy];
        gCanCheck = canCheck;
    }
}

static BOOL FQIsEnabled(void) {
    @synchronized ([NSObject class]) {
        return gEnabled;
    }
}

@interface FQUpdaterDelegate : NSObject
@end

@implementation FQUpdaterDelegate

- (BOOL)updater:(SPUUpdater *)updater mayPerformUpdateCheck:(NSInteger)updateCheck error:(NSError * __autoreleasing _Nullable *)error {
    (void)updater;
    // SPUUpdateCheckUpdates is 0 in Sparkle 2.10. Decline background/probe checks
    // even if a preference is changed outside this bridge while it is running.
    if (FQIsEnabled() && updateCheck == 0) return YES;
    if (error != NULL) {
        *error = [NSError errorWithDomain:@"FourQuadrantsUpdater" code:1 userInfo:@{
            NSLocalizedDescriptionKey: @"Only manual update checks are enabled."
        }];
    }
    return NO;
}

- (BOOL)updaterShouldPromptForPermissionToCheckForUpdates:(SPUUpdater *)updater {
    (void)updater;
    return NO;
}

- (NSArray<NSString *> *)allowedSystemProfileKeysForUpdater:(SPUUpdater *)updater {
    (void)updater;
    return @[];
}

- (void)updater:(SPUUpdater *)updater didFindValidUpdate:(SUAppcastItem *)item {
    if (!FQIsEnabled()) return;
    NSString *version = item.displayVersionString;
    FQSetStatus(@"available", version.length ? [NSString stringWithFormat:@"Version %@ is available.", version] : @"An update is available.", version, updater.canCheckForUpdates);
}

- (void)updaterDidNotFindUpdate:(SPUUpdater *)updater error:(NSError *)error {
    if (!FQIsEnabled()) return;
    gNoUpdateError = error;
    FQSetStatus(@"current", @"No compatible update is currently available.", nil, updater.canCheckForUpdates);
}

- (BOOL)updater:(SPUUpdater *)updater shouldPostponeRelaunchForUpdate:(SUAppcastItem *)item untilInvokingBlock:(void (^)(void))installHandler {
    (void)updater;
    NSString *version = item.displayVersionString;
    @synchronized ([NSObject class]) {
        // Returning YES without invoking the block also fails closed if updates
        // were disabled after this installation cycle began.
        if (!gEnabled) return YES;
        gPostponementObserved = YES;
        gInstallApproved = NO;
        gPendingInstall = [installHandler copy];
        gState = @"install_pending";
        gVersion = [version copy];
        gMessage = @"Save drafts before allowing the update to install and relaunch.";
        gCanCheck = NO;
    }
    return YES;
}

- (BOOL)updaterShouldRelaunchApplication:(SPUUpdater *)updater {
    (void)updater;
    @synchronized ([NSObject class]) {
        if (!gEnabled) return NO;
        if (!FQTerminationGuardIsActive()) {
            gRelaunchBlocked = YES;
            gPendingInstall = nil;
            FQSetStatus(@"error", @"The application's termination guard changed; the update cannot safely relaunch.", nil, NO);
            return NO;
        }
        // Arm the final gate before permitting any path that could skip Sparkle's
        // postponement. Ordinary quits also remain guarded from this point on.
        gInstallStarted = YES;
        // In Sparkle 2.10 SPUInstallerDriver, this gate precedes postponement.
        // _postponedOnce becomes true only when that driver calls our callback;
        // resumed drivers begin false. Never reset our process-wide observation:
        // subsequent attempts may skip postponement and need fresh save approval.
        if (!gPostponementObserved) return YES;
        if (gInstallApproved) {
            gInstallApproved = NO;
            return YES;
        }
        gRelaunchBlocked = YES;
        gPendingInstall = nil; // Sparkle aborts this driver's cycle after NO.
        gState = @"error";
        gMessage = @"Relaunch was blocked because drafts have not been saved for this attempt. Save and restart the application before retrying the update.";
        gCanCheck = NO;
        return NO;
    }
}

- (void)updater:(SPUUpdater *)updater didAbortWithError:(NSError *)error {
    if (!FQIsEnabled() || [error isEqual:gNoUpdateError]) return;
    @synchronized ([NSObject class]) {
        gPendingInstall = nil;
        gInstallApproved = NO;
        if (gRelaunchBlocked) return;
    }
    FQSetStatus(@"error", error.localizedDescription.length ? error.localizedDescription : @"The update check failed.", nil, updater.canCheckForUpdates);
}

- (void)updater:(SPUUpdater *)updater didFinishUpdateCycleForUpdateCheck:(NSInteger)updateCheck error:(NSError * _Nullable)error {
    (void)updateCheck;
    if (!FQIsEnabled()) return;
    @synchronized ([NSObject class]) {
        // A finished cycle invalidates the driver captured weakly by Sparkle's
        // deferred install block. Never retain an inert block for a later retry.
        gPendingInstall = nil;
        gInstallApproved = NO;
        gResumeQueued = NO;
        if (gRelaunchBlocked) return;
        gCanCheck = updater.canCheckForUpdates;
        if (error != nil && ![error isEqual:gNoUpdateError]) {
            gState = @"error";
            gMessage = error.localizedDescription.length ? error.localizedDescription : @"The update check failed.";
            gVersion = nil;
        } else if ([gState isEqualToString:@"checking"] || [gState isEqualToString:@"install_pending"]) {
            gState = @"ready";
            gMessage = @"The update session has finished. Check again to continue.";
            gVersion = nil;
        }
    }
}

@end

static void FQSetError(NSString *message) {
    FQSetStatus(@"error", message, nil, NO);
}

// Must be called on the main thread before any checks. enabled=false latches off
// for this process without loading Sparkle or writing its preferences; Rust must
// pass false on its first call when using an alternate --data store.
void fq_updater_initialize(bool enabled) {
    @synchronized ([NSObject class]) {
        if (!enabled || gPermanentlyDisabled) {
            gEnabled = NO;
            gPermanentlyDisabled = YES;
            gInstallApproved = NO;
            FQSetStatus(@"disabled", @"Updates are disabled for this data store.", nil, NO);
            return;
        }
    }
    if (![NSThread isMainThread]) {
        FQSetError(@"Updater initialization must run on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        gEnabled = YES;
        if (gUpdaterController != nil) return;
    }

    NSBundle *mainBundle = NSBundle.mainBundle;
    if (![mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"]) {
        FQSetStatus(@"unavailable", @"Updates require the packaged macOS application.", nil, NO);
        return;
    }

    id feedURL = [mainBundle objectForInfoDictionaryKey:@"SUFeedURL"];
    id publicKey = [mainBundle objectForInfoDictionaryKey:@"SUPublicEDKey"];
    if (![feedURL isKindOfClass:[NSString class]] || [feedURL length] == 0) {
        FQSetStatus(@"unavailable", @"SUFeedURL is missing from the application Info.plist.", nil, NO);
        return;
    }
    if (![publicKey isKindOfClass:[NSString class]] || [publicKey length] == 0) {
        FQSetStatus(@"unavailable", @"SUPublicEDKey is missing from the application Info.plist.", nil, NO);
        return;
    }

    NSString *frameworksPath = mainBundle.privateFrameworksPath;
    NSString *sparklePath = frameworksPath.length ? [frameworksPath stringByAppendingPathComponent:@"Sparkle.framework"] : nil;
    if (sparklePath == nil || ![[NSFileManager defaultManager] fileExistsAtPath:sparklePath]) {
        FQSetStatus(@"unavailable", @"Sparkle.framework is not embedded in this application.", nil, NO);
        return;
    }

    // Install before loading Sparkle, so no update can start without a final
    // AppKit termination gate. Existing host termination handlers are not replaced.
    if (gTerminationHandler == NULL || !FQInstallTerminationGuard(NSApp)) {
        FQSetStatus(@"unavailable", @"Updates require a registered draft-save handler and a compatible application delegate.", nil, NO);
        return;
    }

    NSError *loadError = nil;
    NSBundle *sparkleBundle = [NSBundle bundleWithPath:sparklePath];
    if (sparkleBundle == nil || ![sparkleBundle loadAndReturnError:&loadError]) {
        NSString *detail = loadError.localizedDescription.length ? loadError.localizedDescription : @"The framework could not be loaded.";
        FQSetStatus(@"unavailable", [NSString stringWithFormat:@"Sparkle.framework is unavailable: %@", detail], nil, NO);
        return;
    }

    Class controllerClass = NSClassFromString(@"SPUStandardUpdaterController");
    if (controllerClass == Nil ||
        ![controllerClass instancesRespondToSelector:@selector(initWithStartingUpdater:updaterDelegate:userDriverDelegate:)] ||
        ![controllerClass instancesRespondToSelector:@selector(updater)] ||
        ![controllerClass instancesRespondToSelector:@selector(checkForUpdates:)]) {
        FQSetStatus(@"unavailable", @"The embedded Sparkle framework does not provide the required Sparkle 2 controller API.", nil, NO);
        return;
    }

    FQUpdaterDelegate *delegate = [[FQUpdaterDelegate alloc] init];
    SPUStandardUpdaterController *controller = [[controllerClass alloc] initWithStartingUpdater:NO updaterDelegate:delegate userDriverDelegate:nil];
    if (controller == nil) {
        FQSetStatus(@"unavailable", @"Sparkle could not create its standard updater controller.", nil, NO);
        return;
    }

    SPUUpdater *updater = controller.updater;
    if (![updater respondsToSelector:@selector(startUpdater:)] ||
        ![updater respondsToSelector:@selector(setAutomaticallyChecksForUpdates:)] ||
        ![updater respondsToSelector:@selector(setAutomaticallyDownloadsUpdates:)] ||
        ![updater respondsToSelector:@selector(setSendsSystemProfile:)] ||
        ![updater respondsToSelector:@selector(canCheckForUpdates)] ||
        ![updater respondsToSelector:@selector(sessionInProgress)]) {
        FQSetStatus(@"unavailable", @"The embedded Sparkle updater API is incompatible.", nil, NO);
        return;
    }
    updater.automaticallyChecksForUpdates = NO;
    updater.automaticallyDownloadsUpdates = NO;
    updater.sendsSystemProfile = NO;

    NSError *startError = nil;
    if (![updater startUpdater:&startError]) {
        NSString *detail = startError.localizedDescription.length ? startError.localizedDescription : @"Sparkle could not start.";
        FQSetStatus(@"unavailable", [NSString stringWithFormat:@"Sparkle could not start: %@", detail], nil, NO);
        return;
    }

    @synchronized ([NSObject class]) {
        gSparkleBundle = sparkleBundle;
        gUpdaterDelegate = delegate;
        gUpdaterController = controller;
        gUpdater = updater;
    }
    FQSetStatus(@"ready", @"Updates are ready for a manual check.", nil, updater.canCheckForUpdates);
}

// Must be called on the main thread. Shows Sparkle's standard check UI, or leaves
// an install_pending state intact so Rust can retry saving drafts.
void fq_updater_check(void) {
    if (![NSThread isMainThread]) {
        FQSetError(@"Update checks must run on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        if (!gEnabled) {
            gState = @"disabled";
            gMessage = @"Updates are disabled for this data store.";
            gCanCheck = NO;
            return;
        }
        if (gResumeQueued || gRelaunchBlocked) return;
        if (gPendingInstall != nil) {
            gState = @"install_pending";
            gMessage = @"Save drafts before allowing the pending update to install and relaunch.";
            gCanCheck = NO;
            return;
        }
    }

    if (gUpdaterController == nil || gUpdater == nil) return;
    if (!gUpdater.canCheckForUpdates) return;
    // During an existing session this action merely focuses Sparkle's UI.
    // Preserve its result (including the no-update NSError) until it completes.
    if (!gUpdater.sessionInProgress) {
        gNoUpdateError = nil;
        FQSetStatus(@"checking", @"Checking for updates…", nil, NO);
    }
    [gUpdaterController checkForUpdates:nil];
}

// May be called from any thread (a background caller reads the last snapshot).
// Main-thread polling also refreshes Sparkle's asynchronous canCheckForUpdates.
// Returns a fresh strdup-owned UTF-8 JSON string; release it with free(3). NULL
// is possible only if memory allocation fails. No Objective-C object is borrowed.
char *fq_updater_status(void) {
    @autoreleasepool {
        NSDictionary *snapshot;
        @synchronized ([NSObject class]) {
            if (NSThread.isMainThread && gEnabled && gUpdater != nil && !gRelaunchBlocked && !gResumeQueued) {
                gCanCheck = gPendingInstall != nil ? [gState isEqualToString:@"error"] : gUpdater.canCheckForUpdates;
            }
            NSMutableDictionary *fields = [@{
                @"state": gState ?: @"unavailable",
                @"message": gMessage ?: @"Updater status is unavailable.",
                @"can_check": @(gCanCheck)
            } mutableCopy];
            if (gVersion != nil) fields[@"version"] = gVersion;
            snapshot = [fields copy];
        }

        NSData *json = [NSJSONSerialization dataWithJSONObject:snapshot options:0 error:NULL];
        NSString *text = json == nil ? nil : [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
        if (text == nil) return strdup("{\"state\":\"error\",\"message\":\"Could not serialize updater status.\",\"can_check\":false}");
        return strdup(text.UTF8String);
    }
}

// Must be called on the main thread after Rust has completed its normal draft-save
// path. allowed=false keeps the pending Sparkle block for another save attempt.
void fq_updater_resume_install(bool allowed) {
    if (![NSThread isMainThread]) {
        FQSetError(@"Update installation must be resumed on the main thread.");
        return;
    }

    @synchronized ([NSObject class]) {
        if (!gEnabled) {
            gState = @"disabled";
            gMessage = @"Updates are disabled for this data store.";
            gCanCheck = NO;
            return;
        }
        if (gResumeQueued || gRelaunchBlocked) return;
        if (gPendingInstall == nil) {
            gState = @"error";
            gMessage = @"There is no pending update installation to resume.";
            gCanCheck = gUpdater.canCheckForUpdates;
            return;
        }
        if (!allowed) {
            gInstallApproved = NO;
            gState = @"error";
            gMessage = @"Draft saving did not complete; the update remains pending. Check again to retry saving.";
            gCanCheck = YES;
            return;
        }
        gResumeQueued = YES;
        gState = @"checking";
        gMessage = @"Drafts are saved; Sparkle is continuing the installation.";
        gCanCheck = NO;
    }

    // Rust calls from inside its workspace update. Defer out of that borrow before
    // Sparkle can send an application termination event; never terminate it here.
    dispatch_async(dispatch_get_main_queue(), ^{
        void (^installHandler)(void) = nil;
        @synchronized ([NSObject class]) {
            if (!gResumeQueued) return;
            gResumeQueued = NO;
            if (!gEnabled || gRelaunchBlocked || gPendingInstall == nil) return;
            if (!FQTerminationGuardIsActive()) {
                gRelaunchBlocked = YES;
                FQSetStatus(@"error", @"The application's termination guard changed; the update cannot safely relaunch.", nil, NO);
                return;
            }
            installHandler = gPendingInstall;
            gPendingInstall = nil;
            gInstallStarted = YES;
            gInstallApproved = YES;
        }
        installHandler();
        // Approval is only valid while this exact continuation is executing.
        // Sparkle 2.10 synchronously consults the relaunch gate in this block.
        @synchronized ([NSObject class]) {
            gInstallApproved = NO;
        }
    });
}

// Main-thread-only. Register before enabled initialization. The function pointer
// must remain valid for the process lifetime and never unwind over the C ABI.
// It synchronously saves drafts on every actual termination attempt after an
// update starts, returning false on save failure, reentrant borrow, or panic.
// No updater lock is held while invoking it. NULL makes termination fail closed.
void fq_updater_set_termination_handler(bool (*handler)(void)) {
    if (!NSThread.isMainThread) return;
    @synchronized ([NSObject class]) {
        gTerminationHandler = handler;
    }
}

// Main-thread-only. Call ONLY after a successful prepare_close and irreversible
// closure of the main workspace, with no editor capable of creating new drafts.
// Its weak Rust entity may disappear now; later quits no longer need that entity.
void fq_updater_workspace_closed(void) {
    if (!NSThread.isMainThread) return;
    @synchronized ([NSObject class]) {
        gWorkspaceClosed = YES;
    }
}
