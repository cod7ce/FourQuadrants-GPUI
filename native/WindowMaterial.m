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

#ifdef FQ_MATERIAL_SELF_TEST
static void require(BOOL condition, NSString *message) {
    if (!condition) {
        fprintf(stderr, "%s\n", message.UTF8String);
        exit(1);
    }
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
        [NSApplication sharedApplication];
        NSUInteger baseline = NSApp.windows.count;
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
             "clear, clamp, invalid inputs, API failures, window release and task-dialog geometry passed");
    }
}
#endif
