//! egui 原生仪表盘窗口
//!
//! 替换 CLI ANSI 终端版本，解决 Windows 控制台 VT/选区/清屏问题。
//! 每 2 秒自动刷新，纯读取共享状态文件，不修改。

use std::time::Instant;

use eframe::egui;
use egui::{Color32, RichText, Vec2};

use crate::state;

/// 仪表盘应用状态
struct DashboardApp {
    /// 上次刷新时间
    last_refresh: Instant,
    /// 缓存的实例数据
    instances: Vec<InstanceRow>,
    /// 系统内存 (total_gb, used_gb)
    sys_mem: (f64, f64),
    /// 逻辑处理器数
    phys_cores: u32,
    /// 注册实例数
    total_instances: usize,
    /// 锁文件状态
    lock_status: String,
}

struct InstanceRow {
    agent_name: String,
    pid: u32,
    cores: String,
    working_set_mb: u64,
    max_mb: u64,
    state: String,
    heartbeat: String,
}

impl DashboardApp {
    fn new() -> Self {
        let mut app = Self {
            last_refresh: Instant::now(),
            instances: Vec::new(),
            sys_mem: (0.0, 0.0),
            phys_cores: 0,
            total_instances: 0,
            lock_status: "—".into(),
        };
        app.refresh_data();
        app
    }

    fn refresh_data(&mut self) {
        self.last_refresh = Instant::now();

        // 系统信息
        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();
        sys.refresh_memory();
        self.sys_mem = (
            sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0),
            sys.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0),
        );

        // 状态文件
        let state = state::read_state_file().unwrap_or_default();
        self.phys_cores = state.global_state.physical_cores;
        self.total_instances = state.instances.len();

        // 排序实例
        let mut sorted: Vec<_> = state.instances.values().collect();
        sorted.sort_by(|a, b| a.agent_name.cmp(&b.agent_name).then(a.pid.cmp(&b.pid)));

        self.instances = sorted
            .iter()
            .map(|inst| {
                let alive = sys
                    .process(sysinfo::Pid::from_u32(inst.pid))
                    .is_some();
                let hb = if inst.last_heartbeat.len() >= 19 {
                    inst.last_heartbeat[11..19].to_string()
                } else {
                    "—".into()
                };
                let name = if inst.agent_name.is_empty() {
                    "—"
                } else {
                    &inst.agent_name
                };
                InstanceRow {
                    agent_name: name.to_string(),
                    pid: inst.pid,
                    cores: inst
                        .bound_cores
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    working_set_mb: inst.working_set_mb,
                    max_mb: inst.max_allowed_memory_mb,
                    state: if alive { "running".into() } else { "dead".into() },
                    heartbeat: hb,
                }
            })
            .collect();

        // 锁文件状态
        let lock_path = state::state_file_path().with_extension("json.lock");
        self.lock_status = if lock_path.exists() {
            "正常".into()
        } else {
            "无锁".into()
        };
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 每 2 秒刷新
        if self.last_refresh.elapsed().as_secs() >= 2 {
            self.refresh_data();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));

        egui::CentralPanel::default().show(ctx, |ui| {
            // 标题
            ui.heading(RichText::new(format!(
                "Agent 资源监控仪表盘 v{}",
                env!("CARGO_PKG_VERSION")
            )).strong());

            ui.separator();

            // 系统信息
            ui.horizontal(|ui| {
                ui.label(format!(
                    "系统内存: {:.1} GB  |  已用: {:.1} GB  |  逻辑处理器: {}",
                    self.sys_mem.0, self.sys_mem.1, self.phys_cores
                ));
            });

            ui.separator();

            // 实例表格
            if self.instances.is_empty() {
                ui.label("(无注册实例 — 等待 Agent 进程启动...)");
            } else {
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 40.0)
                    .show(ui, |ui| {
                        egui::Grid::new("instances")
                            .striped(true)
                            .min_col_width(60.0)
                            .show(ui, |ui| {
                                // 表头
                                ui.label(RichText::new("Agent").strong());
                                ui.label(RichText::new("PID").strong());
                                ui.label(RichText::new("CPU核").strong());
                                ui.label(RichText::new("内存MB").strong());
                                ui.label(RichText::new("最大MB").strong());
                                ui.label(RichText::new("状态").strong());
                                ui.label(RichText::new("心跳").strong());
                                ui.end_row();

                                for row in &self.instances {
                                    let color = if row.state == "dead" {
                                        Color32::RED
                                    } else {
                                        Color32::WHITE
                                    };
                                    ui.label(RichText::new(&row.agent_name).color(color));
                                    ui.label(RichText::new(row.pid.to_string()).color(color));
                                    ui.label(RichText::new(&row.cores).color(color));
                                    ui.label(RichText::new(row.working_set_mb.to_string()).color(color));
                                    ui.label(RichText::new(row.max_mb.to_string()).color(color));
                                    ui.label(RichText::new(&row.state).color(color));
                                    ui.label(RichText::new(&row.heartbeat).color(color));
                                    ui.end_row();
                                }
                            });
                    });
            }

            ui.separator();

            // 底部状态栏
            ui.horizontal(|ui| {
                ui.label(format!(
                    "注册实例: {}  |  锁文件: {}",
                    self.total_instances, self.lock_status
                ));
            });
        });
    }
}

/// 加载系统 CJK 字体（中文显示支持）
fn load_chinese_font() -> Option<egui::FontData> {
    let paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
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
        fonts.font_data.insert("chinese".into(), std::sync::Arc::new(font_data));
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, "chinese".into());
        }
        ctx.set_fonts(fonts);
    }
}

/// 启动 egui 仪表盘窗口
pub fn run() -> std::process::ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(720.0, 480.0))
            .with_resizable(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Agent 资源监控仪表盘",
        options,
        Box::new(|cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Ok(Box::new(DashboardApp::new()))
        }),
    );
    std::process::ExitCode::SUCCESS
}
