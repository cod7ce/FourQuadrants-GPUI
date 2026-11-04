#import <AppKit/AppKit.h>
#import <dlfcn.h>
#import <math.h>
#import <objc/runtime.h>
#import <stdint.h>

static char FQMaterialKey;
static char FQBlurRadiusKey;

typedef int32_t (*FQConnectionFunction)(void);
typedef CGError (*FQBlurFunction)(int32_t, uint32_t, int);

typedef struct {
    FQConnectionFunction defaultConnection;
    FQConnectionFunction mainConnection;
    FQBlurFunction setRadius;
} FQBlurAPI;

// Resolve private WindowServer entry points at runtime, without linking a private
// framework. A missing API or rejected request is a failure, never a fixed-radius
// NSVisualEffectView masquerading as an adjustable blur.
static FQBlurAPI *FQWindowBlurAPI(void) {
    static FQBlurAPI api;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        api.defaultConnection = (FQConnectionFunction)dlsym(RTLD_DEFAULT, "CGSDefaultConnectionForThread");
        api.mainConnection = (FQConnectionFunction)dlsym(RTLD_DEFAULT, "CGSMainConnectionID");
        api.setRadius = (FQBlurFunction)dlsym(RTLD_DEFAULT, "CGSSetWindowBackgroundBlurRadius");
    });
    return &api;
}

static int FQSetWindowBlur(NSWindow *window, int radius) {
    FQBlurAPI *api = FQWindowBlurAPI();
    if (!api->setRadius) return 0;
    int32_t connection = api->defaultConnection ? api->defaultConnection() : 0;
    if (!connection && api->mainConnection) connection = api->mainConnection();
    NSInteger number = window.windowNumber;
    if (!connection || number <= 0 || (uint64_t)number > UINT32_MAX) return 0;
    if (api->setRadius(connection, (uint32_t)number, radius) != kCGErrorSuccess) return 0;
    objc_setAssociatedObject(window, &FQBlurRadiusKey, radius ? @(radius) : nil,
                             OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    return 1;
}

static int FQClearWindowBlur(NSWindow *window) {
    return !objc_getAssociatedObject(window, &FQBlurRadiusKey) || FQSetWindowBlur(window, 0);
}

static void FQRemoveMaterial(NSView *host) {
    NSVisualEffectView *material = objc_getAssociatedObject(host, &FQMaterialKey);
    [material removeFromSuperview];
    objc_setAssociatedObject(host, &FQMaterialKey, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
}

int fq_reduce_motion(void) {
    return NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion ? 1 : 0;
}

// Use the embedded artwork even when running the executable outside an app bundle.
// NSData copies the borrowed Rust bytes; NSApplication retains the decoded image.
int fq_set_application_icon(const unsigned char *bytes, size_t length) {
    if (![NSThread isMainThread] || !NSApp || !bytes || length == 0) return 0;
    NSData *data = [NSData dataWithBytes:bytes length:length];
    NSImage *image = [[NSImage alloc] initWithData:data];
    if (!image || !image.isValid) return 0;
    NSApp.applicationIconImage = image;
    return 1;
}

// NSVisualEffectMaterialSidebar is deliberately opaque on recent macOS
// releases, which hid the desktop behind this window. UnderWindowBackground is
// the material AppKit provides for a transparent, full-window content layer:
// it keeps the desktop readable as a soft color field without washing out the
// foreground text.
static NSVisualEffectMaterial FQSelectedMaterial(void) {
    return NSVisualEffectMaterialUnderWindowBackground;
}

// This bridge runs synchronously on GPUI's main thread. -1 preserves the legacy
// material; explicit radii use an untinted behind-window blur. GPUI controls the
// window's transparency, and Rust draws its independent background opacity.
// WindowServer accepts integer radii: round to nearest within 1..64 for positive
// values, reserving zero for genuinely clear transparency.
int fq_configure_window_material(void *renderViewPointer, int enabled, int appearance, double blurRadius) {
    if (![NSThread isMainThread] || !renderViewPointer || !isfinite(blurRadius) ||
        (blurRadius < 0 && blurRadius != -1)) return 0;
    NSView *renderView = (__bridge NSView *)renderViewPointer;
    NSView *host = renderView.superview;
    NSWindow *window = renderView.window;
    if (!host || !window) return 0;
    if (!enabled) {
        FQRemoveMaterial(host);
        return FQClearWindowBlur(window);
    }
    if (blurRadius >= 0) {
        FQRemoveMaterial(host);
        int radius = blurRadius == 0 ? 0 : (int)lround(fmax(1.0, fmin(blurRadius, 64.0)));
        return FQSetWindowBlur(window, radius);
    }
    if (!FQClearWindowBlur(window)) return 0;
    NSVisualEffectView *material = objc_getAssociatedObject(host, &FQMaterialKey);
    if (!material) {
        material = [[NSVisualEffectView alloc] initWithFrame:host.bounds];
        material.identifier = @"FourQuadrantsWindowMaterial";
        material.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        material.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        material.material = FQSelectedMaterial();
        material.state = NSVisualEffectStateActive;
        [host addSubview:material positioned:NSWindowBelow relativeTo:renderView];
        objc_setAssociatedObject(host, &FQMaterialKey, material, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    material.frame = host.bounds;
    // Set only this view's appearance, keeping GPUI's system-appearance observer intact.
    material.appearance = appearance < 0 ? nil : [NSAppearance appearanceNamed:
        (appearance ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua)];
    return 1;
}

static BOOL FQFiniteRect(NSRect rect) {
    return isfinite(rect.origin.x) && isfinite(rect.origin.y) &&
           isfinite(rect.size.width) && isfinite(rect.size.height) &&
           rect.size.width > 0 && rect.size.height > 0;
}

// Keep the calculation independent of AppKit objects so secondary displays,
// narrow parent windows and the menu bar / Dock insets can be tested directly.
static BOOL FQTaskDialogBounds(NSRect parent, NSRect screen, NSRect visible,
                               NSSize desired, NSRect *result) {
    if (!result || !FQFiniteRect(parent) || !FQFiniteRect(screen) || !FQFiniteRect(visible) ||
        !isfinite(desired.width) || !isfinite(desired.height) ||
        desired.width <= 0 || desired.height <= 0) return NO;
    visible = NSIntersectionRect(screen, visible);
    if (!FQFiniteRect(visible)) return NO;
    CGFloat marginX = fmin(16.0, fmax(0.0, (visible.size.width - 1.0) / 2.0));
    CGFloat marginY = fmin(16.0, fmax(0.0, (visible.size.height - 1.0) / 2.0));
    NSRect usable = NSInsetRect(visible, marginX, marginY);
    NSSize size = NSMakeSize(fmin(desired.width, usable.size.width),
                             fmin(desired.height, usable.size.height));
    NSRect anchor = parent.size.width >= desired.width ? parent : visible;
    CGFloat x = fmax(NSMinX(usable), fmin(NSMidX(anchor) - size.width / 2.0,
                                         NSMaxX(usable) - size.width));
    CGFloat y = fmax(NSMinY(usable), fmin(NSMidY(anchor) - size.height / 2.0,
                                         NSMaxY(usable) - size.height));
    // GPUI 0.2.2 takes display-local coordinates with a top-left origin.
    *result = NSMakeRect(x - NSMinX(screen), NSMaxY(screen) - y - size.height,
                         size.width, size.height);
    return YES;
}

// The view is borrowed only for this synchronous main-thread call. Return
// copied geometry, never an NSWindow/NSScreen pointer to Rust.
int fq_task_dialog_bounds(void *renderViewPointer, double width, double height, double *bounds) {
    if (![NSThread isMainThread] || !renderViewPointer || !bounds) return 0;
    NSView *renderView = (__bridge NSView *)renderViewPointer;
    NSWindow *window = renderView.window;
    NSScreen *screen = window.screen;
    if (!window || !screen) return 0;
    NSRect result;
    if (!FQTaskDialogBounds(window.frame, screen.frame, screen.visibleFrame,
                            NSMakeSize(width, height), &result)) return 0;
    bounds[0] = result.origin.x;
    bounds[1] = result.origin.y;
    bounds[2] = result.size.width;
    bounds[3] = result.size.height;
    return 1;
}

int fq_configure_task_dialog(void *renderViewPointer) {
    if (![NSThread isMainThread] || !renderViewPointer || !NSApp) return 0;
    NSView *renderView = (__bridge NSView *)renderViewPointer;
    NSWindow *window = renderView.window;
    if (![window isKindOfClass:NSPanel.class]) return 0;
    NSPanel *panel = (NSPanel *)window;
    // Preserve GPUIPanel and its delegate, including canBecomeKeyWindow and
    // input dispatch. Changing its nonactivating style after creation can leave
    // WindowServer's activation state inconsistent, so activate the app explicitly.
    panel.floatingPanel = YES;
    panel.level = NSFloatingWindowLevel;
    panel.collectionBehavior = NSWindowCollectionBehaviorMoveToActiveSpace |
                               NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.animationBehavior = NSWindowAnimationBehaviorUtilityWindow;
    panel.becomesKeyOnlyIfNeeded = NO;
    panel.hidesOnDeactivate = YES;
    panel.acceptsMouseMovedEvents = YES;
    // Hide the traffic lights without changing styleMask or the titlebar/frame
    // geometry. GPUI retains its native close handler for the custom close action.
    [panel standardWindowButton:NSWindowCloseButton].hidden = YES;
    [panel standardWindowButton:NSWindowMiniaturizeButton].hidden = YES;
    [panel standardWindowButton:NSWindowZoomButton].hidden = YES;
    [panel standardWindowButton:NSWindowZoomButton].enabled = NO;
    [panel standardWindowButton:NSWindowMiniaturizeButton].enabled = NO;
    // This is an independent panel: do not attach it as a child of the main window.
    [NSApp activateIgnoringOtherApps:YES];
    [panel makeKeyAndOrderFront:nil];
    return [panel makeFirstResponder:renderView] ? 1 : 0;
}

// GPUI 0.2.2 only delivers Dock reopen when AppKit reports no visible windows.
// Island is a visible panel even with the workspace closed. Forward that case as
// having no document windows, keeping GPUI's existing reopen callback and delegate.
static IMP FQOriginalReopen;
static BOOL FQWorkspaceReopen(id delegate, SEL selector, NSApplication *application, BOOL visible) {
    BOOL workspaceVisible = NO;
    for (NSWindow *window in application.windows) {
        if (![window isKindOfClass:NSPanel.class] && window.visible && !window.miniaturized) {
            workspaceVisible = YES;
            break;
        }
    }
    if (FQOriginalReopen) {
        ((void (*)(id, SEL, NSApplication *, BOOL))FQOriginalReopen)(delegate, selector, application, visible && workspaceVisible);
    }
    return YES;
}

void fq_configure_workspace_reopen(void) {
    if (![NSThread isMainThread] || FQOriginalReopen || !NSApp.delegate) return;
    Method method = class_getInstanceMethod([NSApp.delegate class], @selector(applicationShouldHandleReopen:hasVisibleWindows:));
    if (method) FQOriginalReopen = method_setImplementation(method, (IMP)FQWorkspaceReopen);
}

// Keep the workspace and its save/update guards alive when its window is closed.
// Defer ordering out: AppKit can synchronously call GPUI activation callbacks.
int fq_hide_workspace(void *pointer) {
    if (![NSThread isMainThread] || !pointer) return 0;
    NSWindow *window = ((__bridge NSView *)pointer).window;
    if (!window) return 0;
    __weak NSWindow *weakWindow = window;
    dispatch_async(dispatch_get_main_queue(), ^{
        [weakWindow orderOut:nil];
    });
    return 1;
}

// Prefer the physical notch even when the workspace lives on an external display.
// AppKit reports the camera exclusion through public safe-area APIs.
static NSScreen *FQIslandScreen(NSWindow *owner) {
    for (NSScreen *screen in NSScreen.screens) {
        if (screen.safeAreaInsets.top > 0) return screen;
    }
    return owner.screen ?: NSScreen.mainScreen;
}

int fq_island_layout(void *pointer, double *values) {
    if (![NSThread isMainThread] || !pointer || !values) return 0;
    NSScreen *screen = FQIslandScreen(((__bridge NSView *)pointer).window);
    if (!screen) return 0;
    CGFloat height = screen.safeAreaInsets.top;
    NSRect left = screen.auxiliaryTopLeftArea;
    NSRect right = screen.auxiliaryTopRightArea;
    CGFloat width = height > 0 && !NSIsEmptyRect(left) && !NSIsEmptyRect(right)
        ? NSMinX(right) - NSMaxX(left) : 0;
    values[0] = [screen.deviceDescription[@"NSScreenNumber"] unsignedIntValue];
    values[1] = screen.frame.size.width;
    values[2] = NSMaxY(screen.frame) - NSMinY(screen.visibleFrame);
    values[3] = fmax(0, width);
    values[4] = fmax(0, height);
    // Menu-bar height remains available with auto-hide enabled. visibleFrame
    // alone can report a zero top inset, or include a Dock positioned at the top.
    values[5] = NSApp.mainMenu.menuBarHeight > 0
        ? NSApp.mainMenu.menuBarHeight : NSStatusBar.systemStatusBar.thickness;
    return 1;
}

// Island geometry is in global AppKit coordinates; creation converts it to GPUI.
// Anchor to the physical top edge, including the menu-bar/camera strip.
static BOOL FQIslandFrame(NSRect screen, NSRect visible, NSSize desired, NSRect *result) {
    if (!result || !FQFiniteRect(screen) || !FQFiniteRect(visible) ||
        !isfinite(desired.width) || !isfinite(desired.height) ||
        desired.width <= 0 || desired.height <= 0) return NO;
    visible = NSIntersectionRect(screen, visible);
    if (!FQFiniteRect(visible)) return NO;
    NSSize size = NSMakeSize(fmin(desired.width, screen.size.width),
                             fmin(desired.height, NSMaxY(screen) - NSMinY(visible)));
    *result = NSMakeRect(NSMidX(screen) - size.width / 2,
                         NSMaxY(screen) - size.height, size.width, size.height);
    return YES;
}

int fq_island_bounds(void *pointer, double width, double height, double *bounds) {
    if (![NSThread isMainThread] || !pointer || !bounds) return 0;
    NSScreen *screen = FQIslandScreen(((__bridge NSView *)pointer).window);
    NSRect frame;
    if (!screen || !FQIslandFrame(screen.frame, screen.visibleFrame, NSMakeSize(width, height), &frame)) return 0;
    bounds[0] = NSMinX(frame) - NSMinX(screen.frame);
    bounds[1] = NSMaxY(screen.frame) - NSMaxY(frame);
    bounds[2] = frame.size.width;
    bounds[3] = frame.size.height;
    return 1;
}

@interface FQIslandState : NSObject
@property(nonatomic, weak) NSPanel *panel;
@property(nonatomic) NSSize desired;
@property(nonatomic) BOOL queued;
@property(nonatomic) BOOL configured;
@property(nonatomic) BOOL closed;
@property(nonatomic, strong) id closeObserver;
@property(nonatomic, strong) id screenObserver;
@end

@implementation FQIslandState
- (void)dealloc {
    if (_closeObserver) [NSNotificationCenter.defaultCenter removeObserver:_closeObserver];
    if (_screenObserver) [NSNotificationCenter.defaultCenter removeObserver:_screenObserver];
}
@end

static char FQIslandKey;
static void FQQueueIslandFrame(FQIslandState *state) {
    if (state.queued || state.closed) return;
    state.queued = YES;
    // AppKit geometry synchronously calls GPUI's resize/move callbacks. Enqueue
    // the native mutation after Rust releases its window borrow; coalesce requests.
    dispatch_async(dispatch_get_main_queue(), ^{
        state.queued = NO;
        NSPanel *panel = state.panel;
        if (!panel || state.closed) return;
        @try {
            if (!state.configured) {
                // Keep GPUI's NSPanel subclass and the nonactivating bit intact.
                panel.styleMask = NSWindowStyleMaskNonactivatingPanel;
                panel.floatingPanel = YES;
                panel.level = NSMainMenuWindowLevel + 3;
                panel.hidesOnDeactivate = NO;
                // GPUIView does not implement needsPanelToBecomeKey. Permit
                // explicit clicks to make this nonactivating panel key, while
                // orderFront and hover still leave the current key window alone.
                panel.becomesKeyOnlyIfNeeded = NO;
                panel.acceptsMouseMovedEvents = YES;
                panel.movable = NO;
                panel.hasShadow = NO;
                panel.animationBehavior = NSWindowAnimationBehaviorNone;
                panel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                           NSWindowCollectionBehaviorFullScreenAuxiliary;
                state.configured = YES;
            }
            NSScreen *screen = FQIslandScreen(panel);
            NSRect frame;
            if (screen && FQIslandFrame(screen.frame, screen.visibleFrame, state.desired, &frame)) {
                if (!NSEqualRects(panel.frame, frame)) [panel setFrame:frame display:YES];
            }
            if (!panel.visible && !state.closed) [panel orderFront:nil];
        } @catch (NSException *exception) {
            NSLog(@"Island window configuration failed: %@", exception.reason);
        }
    });
}

int fq_configure_island(void *pointer) {
    if (![NSThread isMainThread] || !pointer) return 0;
    NSWindow *window = ((__bridge NSView *)pointer).window;
    if (![window isKindOfClass:NSPanel.class]) return 0;
    FQIslandState *state = objc_getAssociatedObject(window, &FQIslandKey);
    if (!state) {
        state = [FQIslandState new];
        state.panel = (NSPanel *)window;
        state.desired = window.contentView.bounds.size;
        __weak FQIslandState *weakState = state;
        state.closeObserver = [NSNotificationCenter.defaultCenter addObserverForName:NSWindowWillCloseNotification
            object:window queue:nil usingBlock:^(NSNotification *notification) {
                (void)notification;
                weakState.closed = YES;
            }];
        state.screenObserver = [NSNotificationCenter.defaultCenter addObserverForName:NSApplicationDidChangeScreenParametersNotification
            object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *notification) {
                (void)notification;
                FQIslandState *current = weakState;
                if (current) FQQueueIslandFrame(current);
            }];
        objc_setAssociatedObject(window, &FQIslandKey, state, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    FQQueueIslandFrame(state);
    return 1;
}

int fq_resize_island(void *pointer, double width, double height) {
    if (![NSThread isMainThread] || !pointer || !isfinite(width) || !isfinite(height) || width <= 0 || height <= 0) return 0;
    NSWindow *window = ((__bridge NSView *)pointer).window;
    FQIslandState *state = objc_getAssociatedObject(window, &FQIslandKey);
    if (!state || state.closed) return 0;
    state.desired = NSMakeSize(width, height);
    FQQueueIslandFrame(state);
    return 1;
}

#ifdef FQ_MATERIAL_SELF_TEST
static void require(BOOL condition, NSString *message) {
    if (!condition) {
        fprintf(stderr, "%s\n", message.UTF8String);
        exit(1);
    }
}

static void FQTestIslandBounds(void) {
    NSRect screen = NSMakeRect(0, 0, 1440, 900);
    NSRect visible = NSMakeRect(0, 24, 1440, 852);
    NSRect compact, expanded;
    require(FQIslandFrame(screen, visible, NSMakeSize(344, 58), &compact) &&
            NSEqualRects(compact, NSMakeRect(548, 842, 344, 58)), @"Island compact anchor incorrect");
    require(FQIslandFrame(screen, visible, NSMakeSize(640, 408), &expanded) &&
            NSMidX(compact) == NSMidX(expanded) && NSMaxY(compact) == NSMaxY(expanded),
            @"Island expansion moved its top center");
    require(FQIslandFrame(NSMakeRect(-1920, 900, 1920, 1080), NSMakeRect(-1920, 900, 1920, 1056),
                         NSMakeSize(640, 408), &expanded) &&
            NSEqualRects(expanded, NSMakeRect(-1280, 1572, 640, 408)), @"Island secondary-display anchor incorrect");
    require(FQIslandFrame(screen, visible, NSMakeSize(2000, 1200), &expanded) &&
            NSEqualRects(expanded, NSMakeRect(0, 24, 1440, 876)), @"Island exceeded screen or covered the Dock");
    require(!FQIslandFrame(screen, visible, NSMakeSize(NAN, 58), &expanded) &&
            !FQIslandFrame(screen, visible, NSMakeSize(344, 0), &expanded) &&
            !FQIslandFrame(screen, NSZeroRect, NSMakeSize(344, 58), &expanded),
            @"Invalid Island geometry accepted");
}

static void FQTestTaskDialogBounds(void) {
    NSRect screen = NSMakeRect(0, 0, 1440, 900);
    NSRect visible = NSMakeRect(0, 24, 1440, 852);
    NSRect parent = NSMakeRect(200, 140, 1000, 650);
    NSRect result;
    require(FQTaskDialogBounds(parent, screen, visible, NSMakeSize(680, 600), &result) &&
            NSEqualRects(result, NSMakeRect(360, 135, 680, 600)), @"Dialog not centered on parent");
    require(FQTaskDialogBounds(NSMakeRect(100, 200, 400, 400), screen, visible,
                              NSMakeSize(680, 600), &result) &&
            NSEqualRects(result, NSMakeRect(380, 150, 680, 600)), @"Narrow parent did not use screen center");
    require(FQTaskDialogBounds(parent, screen, visible, NSMakeSize(680, 1000), &result) &&
            NSEqualRects(result, NSMakeRect(360, 40, 680, 820)), @"Dialog height not clamped to usable screen");
    require(FQTaskDialogBounds(parent, screen, visible, NSMakeSize(2000, 1200), &result) &&
            NSEqualRects(result, NSMakeRect(16, 40, 1408, 820)), @"Oversized dialog not clamped");
    require(FQTaskDialogBounds(NSMakeRect(1300, 800, 1000, 650), screen, visible,
                              NSMakeSize(680, 600), &result) &&
            NSEqualRects(result, NSMakeRect(744, 40, 680, 600)), @"Dialog escaped visible screen");
    require(FQTaskDialogBounds(NSMakeRect(-1700, 1100, 1000, 650),
                              NSMakeRect(-1920, 900, 1920, 1080),
                              NSMakeRect(-1920, 900, 1920, 1056), NSMakeSize(680, 600), &result) &&
            NSEqualRects(result, NSMakeRect(380, 255, 680, 600)), @"Left/upper display coordinates incorrect");
    require(FQTaskDialogBounds(NSMakeRect(1660, -880, 1000, 650),
                              NSMakeRect(1440, -1080, 1920, 1080),
                              NSMakeRect(1440, -1080, 1920, 1056), NSMakeSize(680, 600), &result) &&
            NSEqualRects(result, NSMakeRect(380, 255, 680, 600)), @"Right/lower display coordinates incorrect");
    require(!FQTaskDialogBounds(parent, screen, visible, NSMakeSize(NAN, 600), &result) &&
            !FQTaskDialogBounds(parent, screen, visible, NSMakeSize(680, 0), &result) &&
            !FQTaskDialogBounds(parent, screen, NSZeroRect, NSMakeSize(680, 600), &result),
            @"Invalid dialog geometry accepted");
}

static void FQDrainIsland(FQIslandState *state) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:1];
    while (state.queued && deadline.timeIntervalSinceNow > 0) {
        [NSRunLoop.mainRunLoop runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
    require(!state.queued, @"Island queued mutation did not finish");
}

static void FQTestIslandPanel(void) {
    __weak NSPanel *released;
    @autoreleasepool {
        NSPanel *panel = [[NSPanel alloc] initWithContentRect:NSMakeRect(100, 100, 344, 58)
            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskNonactivatingPanel | NSWindowStyleMaskFullSizeContentView
            backing:NSBackingStoreBuffered defer:NO];
        panel.releasedWhenClosed = NO;
        released = panel;
        void *view = (__bridge void *)panel.contentView;
        NSWindow *key = NSApp.keyWindow;
        require(fq_configure_island(view), @"Island setup rejected panel");
        FQIslandState *state = objc_getAssociatedObject(panel, &FQIslandKey);
        require(!state.configured && state.queued, @"Island changed geometry synchronously");
        FQDrainIsland(state);
        require(state.configured && !panel.hidesOnDeactivate && !panel.becomesKeyOnlyIfNeeded &&
                panel.level == NSMainMenuWindowLevel + 3 && panel.styleMask == NSWindowStyleMaskNonactivatingPanel,
                @"Island native policy incorrect");
        require(NSApp.keyWindow == key, @"Island setup stole keyboard focus");
        NSRect before = panel.frame;
        require(fq_resize_island(view, 640, 408) && fq_resize_island(view, 420, 160), @"Island resize failed");
        require(NSEqualRects(panel.frame, before), @"Island resized inside borrowed window callback");
        FQDrainIsland(state);
        require(panel.frame.size.width == 420 && panel.frame.size.height == 160 &&
                NSMidX(panel.frame) == NSMidX(before) && NSMaxY(panel.frame) == NSMaxY(before),
                @"Island failed to coalesce resize or lost anchor");
        require(!fq_resize_island(view, NAN, 100) && !fq_resize_island(view, 100, 0), @"Island accepted invalid resize");
        require(fq_resize_island(view, 344, 58), @"Island final resize failed");
        [panel close];
        FQDrainIsland(state);
        require(state.closed && !panel.visible && !fq_resize_island(view, 640, 408), @"Closed Island was reopened by queued work");
    }
    require(!released, @"Island panel leaked");
}

static CGError FQRejectedBlur(int32_t connection, uint32_t window, int radius) {
    (void)connection;
    (void)window;
    (void)radius;
    return kCGErrorFailure;
}

static int32_t FQMissingConnection(void) {
    return 0;
}

static void FQTestMaterial(NSWindow *window) {
    NSView *host = window.contentView;
    NSView *render = [[NSView alloc] initWithFrame:host.bounds];
    render.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [host addSubview:render];
    void *pointer = (__bridge void *)render;
    NSUInteger baseline = host.subviews.count;
    require(fq_configure_window_material(pointer, 1, 1, -1), @"Legacy enable failed");
    NSVisualEffectView *material = objc_getAssociatedObject(host, &FQMaterialKey);
    require(material.material == NSVisualEffectMaterialUnderWindowBackground, @"Wrong material");
    require(material.blendingMode == NSVisualEffectBlendingModeBehindWindow, @"Wrong blending mode");
    require(material.state == NSVisualEffectStateActive, @"Inactive material");
    require([material.appearance.name isEqual:NSAppearanceNameDarkAqua], @"Dark appearance missing");
    require([host.subviews indexOfObject:material] < [host.subviews indexOfObject:render], @"Material obscures renderer");
    require(fq_configure_window_material(pointer, 1, 0, -1), @"Legacy update failed");
    require(host.subviews.count == baseline + 1, @"Duplicate material after update");
    require([material.appearance.name isEqual:NSAppearanceNameAqua], @"Light appearance missing");
    [window setContentSize:NSMakeSize(800, 600)];
    require(NSEqualRects(material.frame, host.bounds), @"Material did not resize");
    require(fq_configure_window_material(pointer, 1, -1, -1), @"Legacy system appearance failed");
    require(material.appearance == nil, @"System appearance did not inherit");
    require(fq_configure_window_material(pointer, 0, -1, -1), @"Legacy disable failed");
    require(host.subviews.count == baseline && render.superview == host, @"Disable removed renderer");
    require(!objc_getAssociatedObject(host, &FQMaterialKey), @"Material not released");
    require(fq_configure_window_material(pointer, 0, -1, -1), @"Repeated legacy disable failed");
    require(host.subviews.count == baseline, @"Disable is not idempotent");

    // Exercise actual WindowServer transitions, including removing the legacy tint.
    require(fq_configure_window_material(pointer, 1, 1, -1), @"Legacy re-enable failed");
    int radii[] = {0, 12, 48, 64, 0, 64, 12};
    for (size_t i = 0; i < sizeof(radii) / sizeof(radii[0]); ++i) {
        require(fq_configure_window_material(pointer, 1, 1, radii[i]),
                [NSString stringWithFormat:@"WindowServer rejected radius %d", radii[i]]);
        require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == radii[i], @"Wrong stored radius");
        require(host.subviews.count == baseline && !objc_getAssociatedObject(host, &FQMaterialKey),
                @"Explicit blur retained the legacy tint");
        require(render.superview == host && window.alphaValue == 1 && !window.opaque &&
                [window.backgroundColor isEqual:NSColor.clearColor], @"Explicit blur changed renderer opacity");
    }
    require(fq_configure_window_material(pointer, 1, -1, 1000), @"Radius clamp failed");
    require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == 64, @"Radius not clamped to 64");
    require(fq_configure_window_material(pointer, 1, -1, 0.1), @"Small positive radius failed");
    require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == 1, @"Positive radius became clear");
    require(fq_configure_window_material(pointer, 1, -1, 12.5), @"Fractional radius failed");
    require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == 13, @"Fractional radius not rounded");
    double invalid[] = {NAN, INFINITY, -INFINITY, -2, -0.5};
    for (size_t i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        require(!fq_configure_window_material(pointer, 1, -1, invalid[i]), @"Invalid radius accepted");
        require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == 13, @"Invalid radius mutated blur");
    }
    require(!fq_configure_window_material(NULL, 1, 0, 12), @"Null render view accepted");
    NSView *detached = [[NSView alloc] initWithFrame:NSZeroRect];
    require(!fq_configure_window_material((__bridge void *)detached, 1, 0, 12), @"Detached view accepted");
    dispatch_semaphore_t backgroundDone = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        require(!fq_configure_window_material(pointer, 1, 0, 12), @"Background thread accepted");
        dispatch_semaphore_signal(backgroundDone);
    });
    require(dispatch_semaphore_wait(backgroundDone, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) == 0,
            @"Background-thread test timed out");
    require(fq_configure_window_material(pointer, 1, 0, -1), @"Explicit to legacy transition failed");
    require(!objc_getAssociatedObject(window, &FQBlurRadiusKey), @"Legacy transition retained explicit blur");
    require(host.subviews.count == baseline + 1, @"Legacy transition missing material");
    require(fq_configure_window_material(pointer, 1, 0, 48), @"Legacy to explicit transition failed");
    require(fq_configure_window_material(pointer, 0, 0, -1), @"Explicit disable failed");
    require(!objc_getAssociatedObject(window, &FQBlurRadiusKey) && host.subviews.count == baseline,
            @"Disable retained blur or material");
    require(fq_configure_window_material(pointer, 0, 0, 64), @"Repeated explicit disable failed");

    // Verify unavailable APIs and WindowServer errors cannot report success.
    FQBlurAPI *api = FQWindowBlurAPI();
    FQBlurAPI saved = *api;
    api->setRadius = NULL;
    require(fq_configure_window_material(pointer, 1, 0, -1), @"Legacy mode depends on private API");
    require(!fq_configure_window_material(pointer, 1, 0, 12), @"Missing blur API reported success");
    require(!objc_getAssociatedObject(host, &FQMaterialKey), @"Unsupported explicit blur retained tint");
    require(!fq_configure_window_material(pointer, 1, 0, 0), @"Missing API accepted explicit zero");
    *api = saved;
    api->defaultConnection = FQMissingConnection;
    if (api->mainConnection) {
        require(fq_configure_window_material(pointer, 1, 0, 12), @"Main connection fallback failed");
    }
    api->mainConnection = FQMissingConnection;
    require(!fq_configure_window_material(pointer, 1, 0, 48), @"Missing connection reported success");
    *api = saved;
    require(fq_configure_window_material(pointer, 1, 0, 48), @"Failure test setup failed");
    api->setRadius = FQRejectedBlur;
    require(!fq_configure_window_material(pointer, 1, 0, 64), @"WindowServer error reported success");
    require(!fq_configure_window_material(pointer, 0, 0, 0), @"Failed blur clear reported success");
    require(!fq_configure_window_material(pointer, 1, 0, -1), @"Failed clear switched to legacy material");
    require([objc_getAssociatedObject(window, &FQBlurRadiusKey) intValue] == 48, @"Failed request changed recorded radius");
    *api = saved;
    require(fq_configure_window_material(pointer, 1, 0, 0), @"Explicit zero failed to clear previous blur");
    require(!objc_getAssociatedObject(window, &FQBlurRadiusKey), @"Explicit zero retained blur state");
}

int main(void) {
    @autoreleasepool {
        FQTestTaskDialogBounds();
        FQTestIslandBounds();
        [NSApplication sharedApplication];
        NSUInteger baseline = NSApp.windows.count;
        FQTestIslandPanel();
        __weak NSWindow *releasedWindow;
        @autoreleasepool {
            NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 640, 480)
                styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskResizable
                backing:NSBackingStoreBuffered defer:NO];
            window.releasedWhenClosed = NO;
            window.opaque = NO;
            window.backgroundColor = NSColor.clearColor;
            releasedWindow = window;
            FQTestMaterial(window);
            [window close];
        }
        require(!releasedWindow, @"Test window leaked");
        require(NSApp.windows.count == baseline, @"Test left an application window behind");
        puts("Native material: legacy lifecycle, theme, resize, layering; real CGS radii 0/12/48/64; "
             "clear, clamp, invalid inputs, API failures, window release, task-dialog geometry; Island nonactivation, resize coalescing, close lifecycle and geometry passed");
    }
}
#endif
