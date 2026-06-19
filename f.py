#!/usr/bin/env python3
"""Fix: add Chinese font support to egui dashboard"""
with open('/root/my_build/agent-launcher-safe/src/dashboard.rs', 'r') as f:
    c = f.read()

# 1. Add font functions before pub fn run()
old_run = '''/// 启动 egui 仪表盘窗口
pub fn run()'''

new_code = '''/// 加载系统 CJK 字体（中文显示支持）
fn load_chinese_font() -> Option<egui::FontData> {
    let paths = [
        "C:\\\\Windows\\\\Fonts\\\\msyh.ttc",
        "C:\\\\Windows\\\\Fonts\\\\simsun.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    for path in &paths {
        if let Ok(data) = std::fs::read(path) {
            log::info!("加载中文字体: {}", path);
            return Some(egui::FontData::from_owned(data));
        }
    }
    log::warn!("未找到中文字体，中文可能显示为口");
    None
}

/// 配置 egui 字体以支持中文
fn setup_chinese_fonts(ctx: &egui::Context) {
    if let Some(font_data) = load_chinese_font() {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("chinese".into(), font_data);
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, "chinese".into());
        }
        ctx.set_fonts(fonts);
    }
}

/// 启动 egui 仪表盘窗口
pub fn run()'''

assert old_run in c, 'pub fn run not found!'
c = c.replace(old_run, new_code)

# 2. Change run() to use cc callback for font setup
old_fn = '''pub fn run() -> std::process::ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(720.0, 480.0))
            .with_resizable(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Agent 资源监控仪表盘",
        options,
        Box::new(|_cc| Ok(Box::new(DashboardApp::new()))),
    );'''

new_fn = '''pub fn run() -> std::process::ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(720.0, 480.0))
            .with_resizable(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Agent \u8d44\u6e90\u76d1\u63a7\u4eea\u8868\u76d8",
        options,
        Box::new(|cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Ok(Box::new(DashboardApp::new()))
        }),
    );'''

assert old_fn in c, 'run function not found!'
c = c.replace(old_fn, new_fn)

with open('/root/my_build/agent-launcher-safe/src/dashboard.rs', 'w') as f:
    f.write(c)
print('fixed: Chinese font support added')
