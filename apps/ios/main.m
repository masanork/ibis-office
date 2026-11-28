#import <UIKit/UIKit.h>
#import <QuartzCore/CAMetalLayer.h>
#import <CoreText/CoreText.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#include <stdlib.h>
#include <string.h>

extern void ibis_ios_start(void *ui_view, float logical_width, float logical_height, float scale_factor);
extern void ibis_ios_tick(void);
extern void ibis_ios_resize(float logical_width, float logical_height, float scale_factor);
extern void ibis_ios_touch(uint64_t id, uint8_t phase, float x, float y);
extern void ibis_ios_pinch(float x, float y, float delta, uint8_t phase);
extern void ibis_ios_lifecycle(uint8_t phase);
extern void ibis_ios_memory_warning(void);
extern void ibis_ios_on_file_picked(int64_t request_id, const char *path);
extern void ibis_ios_on_save_path_picked(int64_t request_id, const char *path);
extern void ibis_ios_on_open_path(const char *path);
extern void ibis_ios_ime_commit(const char *text);
extern void ibis_ios_ime_backspace(void);
extern void ibis_ios_ime_enter(void);

@interface IbisMetalView : UIView <UIKeyInput>
@property (nonatomic, assign) CGFloat lastPinchScale;
@end

static IbisMetalView *g_metalView = nil;
static UIViewController *g_rootVC = nil;
static NSMutableDictionary<NSString *, NSURL *> *g_exportUrlByTempPath = nil;

@implementation IbisMetalView

+ (Class)layerClass {
    return [CAMetalLayer class];
}

- (instancetype)initWithFrame:(CGRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        self.multipleTouchEnabled = YES;
        self.contentScaleFactor = [UIScreen mainScreen].scale;
        CAMetalLayer *metalLayer = (CAMetalLayer *)self.layer;
        metalLayer.contentsScale = self.contentScaleFactor;

        UIPinchGestureRecognizer *pinch = [[UIPinchGestureRecognizer alloc] initWithTarget:self action:@selector(handlePinch:)];
        [self addGestureRecognizer:pinch];
    }
    return self;
}

- (BOOL)canBecomeFirstResponder {
    return YES;
}

- (BOOL)hasText {
    return YES;
}

- (void)insertText:(NSString *)text {
    if ([text isEqualToString:@"\n"]) {
        ibis_ios_ime_enter();
    } else if (text.length > 0) {
        ibis_ios_ime_commit([text UTF8String]);
    }
}

- (void)deleteBackward {
    ibis_ios_ime_backspace();
}

- (void)layoutSubviews {
    [super layoutSubviews];
    CGSize size = self.bounds.size;
    if (size.width > 0 && size.height > 0) {
        CGFloat scale = self.contentScaleFactor > 0 ? self.contentScaleFactor : [UIScreen mainScreen].scale;
        CAMetalLayer *metalLayer = (CAMetalLayer *)self.layer;
        metalLayer.contentsScale = scale;
        metalLayer.drawableSize = CGSizeMake(size.width * scale, size.height * scale);
        ibis_ios_resize((float)size.width, (float)size.height, (float)scale);
    }
}

- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    for (UITouch *touch in touches) {
        CGPoint pt = [touch locationInView:self];
        ibis_ios_touch((uint64_t)(uintptr_t)touch, 0, (float)pt.x, (float)pt.y);
    }
}

- (void)touchesMoved:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    for (UITouch *touch in touches) {
        CGPoint pt = [touch locationInView:self];
        ibis_ios_touch((uint64_t)(uintptr_t)touch, 1, (float)pt.x, (float)pt.y);
    }
}

- (void)touchesEnded:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    for (UITouch *touch in touches) {
        CGPoint pt = [touch locationInView:self];
        ibis_ios_touch((uint64_t)(uintptr_t)touch, 2, (float)pt.x, (float)pt.y);
    }
}

- (void)touchesCancelled:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event {
    for (UITouch *touch in touches) {
        CGPoint pt = [touch locationInView:self];
        ibis_ios_touch((uint64_t)(uintptr_t)touch, 3, (float)pt.x, (float)pt.y);
    }
}

- (void)handlePinch:(UIPinchGestureRecognizer *)gesture {
    CGPoint center = [gesture locationInView:self];
    if (gesture.state == UIGestureRecognizerStateBegan) {
        self.lastPinchScale = 1.0;
        ibis_ios_pinch((float)center.x, (float)center.y, 0.0f, 0);
    } else if (gesture.state == UIGestureRecognizerStateChanged) {
        CGFloat prev = self.lastPinchScale > 0.0 ? self.lastPinchScale : 1.0;
        float delta = (float)(gesture.scale / prev - 1.0);
        self.lastPinchScale = gesture.scale;
        ibis_ios_pinch((float)center.x, (float)center.y, delta, 1);
    } else if (gesture.state == UIGestureRecognizerStateEnded) {
        ibis_ios_pinch((float)center.x, (float)center.y, 0.0f, 2);
    } else if (gesture.state == UIGestureRecognizerStateCancelled || gesture.state == UIGestureRecognizerStateFailed) {
        ibis_ios_pinch((float)center.x, (float)center.y, 0.0f, 3);
    }
}

@end

@interface IbisViewController : UIViewController <UIDocumentPickerDelegate>
@property (nonatomic, strong) CADisplayLink *displayLink;
@property (nonatomic, assign) BOOL started;
@property (nonatomic, assign) int64_t pendingOpenRequestId;
@property (nonatomic, assign) int64_t pendingSaveRequestId;
@property (nonatomic, assign) BOOL pickingForSave;
@end

static NSString *CopyUrlToOpenedStorage(NSURL *url) {
    BOOL scoped = [url startAccessingSecurityScopedResource];
    NSString *name = url.lastPathComponent ?: @"document";
    NSArray<NSString *> *paths = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES);
    NSString *docs = paths.firstObject ?: NSTemporaryDirectory();
    NSString *dir = [docs stringByAppendingPathComponent:@"opened"];
    [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
    NSString *dest = [dir stringByAppendingPathComponent:[NSString stringWithFormat:@"%lld_%@", (long long)([[NSDate date] timeIntervalSince1970] * 1000), name]];
    NSError *err = nil;
    [[NSFileManager defaultManager] removeItemAtPath:dest error:nil];
    BOOL ok = [[NSFileManager defaultManager] copyItemAtURL:url toURL:[NSURL fileURLWithPath:dest] error:&err];
    if (scoped) [url stopAccessingSecurityScopedResource];
    if (!ok) {
        NSLog(@"Failed to copy picked document: %@", err);
        return nil;
    }
    if (!g_exportUrlByTempPath) g_exportUrlByTempPath = [NSMutableDictionary new];
    g_exportUrlByTempPath[dest] = url;
    return dest;
}

@implementation IbisViewController

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = [UIColor whiteColor];

    IbisMetalView *metalView = [[IbisMetalView alloc] initWithFrame:self.view.bounds];
    metalView.translatesAutoresizingMaskIntoConstraints = NO;
    [self.view addSubview:metalView];
    UILayoutGuide *safe = self.view.safeAreaLayoutGuide;
    [NSLayoutConstraint activateConstraints:@[
        [metalView.topAnchor constraintEqualToAnchor:safe.topAnchor],
        [metalView.bottomAnchor constraintEqualToAnchor:safe.bottomAnchor],
        [metalView.leadingAnchor constraintEqualToAnchor:safe.leadingAnchor],
        [metalView.trailingAnchor constraintEqualToAnchor:safe.trailingAnchor],
    ]];
    g_metalView = metalView;
    g_rootVC = self;

    self.displayLink = [CADisplayLink displayLinkWithTarget:self selector:@selector(onFrame:)];
    [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
}

- (void)viewDidLayoutSubviews {
    [super viewDidLayoutSubviews];
    if (!self.started && g_metalView.bounds.size.width > 0 && g_metalView.bounds.size.height > 0) {
        self.started = YES;
        CGFloat scale = g_metalView.contentScaleFactor > 0 ? g_metalView.contentScaleFactor : [UIScreen mainScreen].scale;
        ibis_ios_start((__bridge void *)g_metalView, (float)g_metalView.bounds.size.width, (float)g_metalView.bounds.size.height, (float)scale);
    }
}

- (void)onFrame:(CADisplayLink *)link {
    if (self.started) {
        ibis_ios_tick();
    }
}

- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    NSURL *url = urls.firstObject;
    if (self.pickingForSave) {
        int64_t reqId = self.pendingSaveRequestId;
        self.pendingSaveRequestId = 0;
        self.pickingForSave = NO;
        ibis_ios_on_save_path_picked(reqId, NULL);
        (void)url;
    } else {
        int64_t reqId = self.pendingOpenRequestId;
        self.pendingOpenRequestId = 0;
        NSString *path = url ? CopyUrlToOpenedStorage(url) : nil;
        ibis_ios_on_file_picked(reqId, path ? [path UTF8String] : NULL);
    }
}

- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller {
    if (self.pickingForSave) {
        int64_t reqId = self.pendingSaveRequestId;
        self.pendingSaveRequestId = 0;
        self.pickingForSave = NO;
        ibis_ios_on_save_path_picked(reqId, NULL);
    } else {
        int64_t reqId = self.pendingOpenRequestId;
        self.pendingOpenRequestId = 0;
        ibis_ios_on_file_picked(reqId, NULL);
    }
}

@end

@interface IbisSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation IbisSceneDelegate

- (void)scene:(UIScene *)scene willConnectToSession:(UISceneSession *)session options:(UISceneConnectionOptions *)connectionOptions {
    if (![scene isKindOfClass:[UIWindowScene class]]) {
        return;
    }
    UIWindowScene *windowScene = (UIWindowScene *)scene;
    self.window = [[UIWindow alloc] initWithWindowScene:windowScene];
    self.window.rootViewController = [[IbisViewController alloc] init];
    [self.window makeKeyAndVisible];

    for (UIOpenURLContext *ctx in connectionOptions.URLContexts) {
        NSString *path = CopyUrlToOpenedStorage(ctx.URL);
        if (path) {
            ibis_ios_on_open_path([path UTF8String]);
        }
    }
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
    for (UIOpenURLContext *ctx in URLContexts) {
        NSString *path = CopyUrlToOpenedStorage(ctx.URL);
        if (path) {
            ibis_ios_on_open_path([path UTF8String]);
        }
    }
}

- (void)sceneDidBecomeActive:(UIScene *)scene {
    ibis_ios_lifecycle(0);
}

- (void)sceneWillResignActive:(UIScene *)scene {
    ibis_ios_lifecycle(1);
}

- (void)sceneDidEnterBackground:(UIScene *)scene {
    ibis_ios_lifecycle(2);
}

- (void)sceneWillEnterForeground:(UIScene *)scene {
    ibis_ios_lifecycle(3);
}

@end

@interface IbisAppDelegate : UIResponder <UIApplicationDelegate>
@end

@implementation IbisAppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    NSArray<NSString *> *langs = [NSLocale preferredLanguages];
    if (langs.count > 0) {
        NSString *lang = [langs.firstObject stringByReplacingOccurrencesOfString:@"-" withString:@"_"];
        setenv("LANG", [[lang stringByAppendingString:@".UTF-8"] UTF8String], 1);
    }
    return YES;
}

- (UISceneConfiguration *)application:(UIApplication *)application configurationForConnectingSceneSession:(UISceneSession *)connectingSceneSession options:(UISceneConnectionOptions *)options {
    return [[UISceneConfiguration alloc] initWithName:@"Default Configuration" sessionRole:connectingSceneSession.role];
}

- (void)applicationDidReceiveMemoryWarning:(UIApplication *)application {
    ibis_ios_memory_warning();
}

@end

void ibis_ios_wake_main(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        ibis_ios_tick();
    });
}

void ibis_ios_show_keyboard(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [g_metalView becomeFirstResponder];
    });
}

void ibis_ios_hide_keyboard(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [g_metalView resignFirstResponder];
    });
}

void ibis_ios_open_url(const char *url) {
    if (!url) return;
    NSString *str = [NSString stringWithUTF8String:url];
    dispatch_async(dispatch_get_main_queue(), ^{
        NSURL *nsurl = [NSURL URLWithString:str];
        if (nsurl) {
            [[UIApplication sharedApplication] openURL:nsurl options:@{} completionHandler:nil];
        }
    });
}

char *ibis_ios_get_clipboard_text(void) {
    NSString *text = [UIPasteboard generalPasteboard].string;
    if (!text) return NULL;
    return strdup([text UTF8String]);
}

void ibis_ios_free_c_string(char *ptr) {
    free(ptr);
}

char *ibis_ios_find_font_path(const char *name) {
    if (!name) return NULL;
    CFStringRef cfName = CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    if (!cfName) return NULL;
    CTFontRef font = CTFontCreateWithName(cfName, 14.0, NULL);
    CFRelease(cfName);
    if (!font) return NULL;
    CFURLRef url = (CFURLRef)CTFontCopyAttribute(font, kCTFontURLAttribute);
    CFRelease(font);
    if (!url) return NULL;
    NSString *path = [(__bridge NSURL *)url path];
    CFRelease(url);
    if (!path) return NULL;
    return strdup([path UTF8String]);
}

void ibis_ios_set_clipboard_text(const char *text) {
    if (!text) return;
    NSString *str = [NSString stringWithUTF8String:text];
    dispatch_async(dispatch_get_main_queue(), ^{
        [UIPasteboard generalPasteboard].string = str;
    });
}

void ibis_ios_pick_file_for_open(int64_t request_id) {
    dispatch_async(dispatch_get_main_queue(), ^{
        IbisViewController *vc = (IbisViewController *)g_rootVC;
        if (!vc) {
            ibis_ios_on_file_picked(request_id, NULL);
            return;
        }
        vc.pendingOpenRequestId = request_id;
        vc.pickingForSave = NO;
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc] initForOpeningContentTypes:@[UTTypeItem]];
        picker.delegate = vc;
        [vc presentViewController:picker animated:YES completion:nil];
    });
}

void ibis_ios_pick_path_for_save(int64_t request_id, const char *suggested_name) {
    NSString *name = suggested_name ? [NSString stringWithUTF8String:suggested_name] : @"document.docx";
    NSArray<NSString *> *paths = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES);
    NSString *docs = paths.firstObject ?: NSTemporaryDirectory();
    NSString *dest = [docs stringByAppendingPathComponent:name];
    ibis_ios_on_save_path_picked(request_id, [dest UTF8String]);
}

bool ibis_ios_sync_saved_file(const char *path) {
    if (!path) return false;
    NSString *localPath = [NSString stringWithUTF8String:path];
    NSURL *url = g_exportUrlByTempPath[localPath];
    if (!url) return false;
    BOOL scoped = [url startAccessingSecurityScopedResource];
    NSData *data = [NSData dataWithContentsOfFile:localPath];
    BOOL ok = NO;
    if (data) {
        ok = [data writeToURL:url atomically:YES];
    }
    if (scoped) [url stopAccessingSecurityScopedResource];
    return ok;
}

int main(int argc, char *argv[]) {
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil, NSStringFromClass([IbisAppDelegate class]));
    }
}
