#import <AppKit/AppKit.h>

// mailfabrica_set_dock_visible 只改变应用展示策略，不退出进程或销毁窗口。
int mailfabrica_set_dock_visible(int visible) {
    NSApplicationActivationPolicy policy = visible ? NSApplicationActivationPolicyRegular : NSApplicationActivationPolicyAccessory;
    if (NSApp.activationPolicy == policy) {
        return 1;
    }
    return [NSApp setActivationPolicy:policy];
}

// mailfabrica_dock_visible 区分正常窗口模式与仅保留菜单栏的后台模式。
int mailfabrica_dock_visible(void) {
    return NSApp.activationPolicy == NSApplicationActivationPolicyRegular;
}
