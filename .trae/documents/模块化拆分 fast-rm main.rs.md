## 目标
- 将当前单文件实现拆分为职责清晰的模块，降低复杂度、提高可维护性与测试性。
- 保持现有行为与 CLI 接口不变；不更改输出格式与并发策略。

## 拆分后的文件结构
- `src/main.rs`：二进制入口，仅负责组装流程与启动。
- `src/cli.rs`：命令行解析结构体与 `Cli::parse()` 封装。
- `src/errors.rs`：`RemoveError` 枚举与 `Display` 实现。
- `src/config.rs`：`RemoveConfig` 与日志方法（`log_action`/`log_check`）。
- `src/progress.rs`：`RemoveProgress` 与 `ProgressDisplay`（TUI）。
- `src/path.rs`：路径去重与重叠检测（`deduplicate_and_check_paths`）。
- `src/removal.rs`：删除主逻辑（`fast_remove`、`remove_file`、`remove_symlink`、`remove_directory`）。
- `src/results.rs`：结果归并与退出（`process_results`、`print_summary_and_exit`）。
- `tests/`（可选增强）：按阶段拆分集成测试（dry-run/真实删除），保留安全护栏。

## 模块职责与依赖关系
- `cli`：仅依赖 `clap`，对外暴露 `Cli`。
- `errors`：标准库 `io`，仅暴露 `RemoveError` 与其 `Display`。
- `config`：依赖 `colored` 与 `Verbosity`，对外暴露 `RemoveConfig` 与日志辅助。
- `progress`：依赖 `indicatif`、`crossterm`，对外暴露 `RemoveProgress`/`ProgressDisplay`。
- `path`：标准库路径与集合，暴露 `deduplicate_and_check_paths`。
- `removal`：依赖 `std::fs`、`rayon::prelude::*`、`errors`、`config`、`progress`；暴露删除相关函数。与 `progress` 仅通过 `RemoveConfig.progress: Option<Arc<RemoveProgress>>` 交互。
- `results`：依赖 `colored` 与 `errors`；对外暴露汇总与退出方法。
- `main`：组装线程池、TUI 线程、并行删除与最终汇总；调用上述各模块的公开 API。

## 可见性与统一命名
- `pub`：跨模块需要的类型与函数（如 `Cli`、`RemoveError`、`RemoveConfig`、`RemoveProgress`、`ProgressDisplay`、`fast_remove`、`process_results`、`print_summary_and_exit`、`deduplicate_and_check_paths`）。
- `pub(crate)`/私有：仅模块内使用的辅助函数与字段保持私有，避免泄漏实现细节。
- 保留现有命名与行为，避免破坏外部用户认知。

## 主文件骨架（示意）
- `main.rs` 仅包含：
  - 解析 CLI 与可选线程池设置（`rayon::ThreadPoolBuilder`）。
  - 调用 `path::deduplicate_and_check_paths`。
  - 创建 `progress::RemoveProgress` 与 `progress::ProgressDisplay`，拼装 `config::RemoveConfig`。
  - 启动 TUI 更新线程（持有 `Arc<ProgressDisplay>` 与 `Arc<RemoveProgress>`）。
  - 使用 `rayon::par_iter()` 并行调用 `removal::fast_remove`。
  - 停止 TUI、调用 `results::process_results`，最后 `results::print_summary_and_exit`。

## 代码迁移映射（来源行号）
- `Cli` → `src/cli.rs`（`src/main.rs:380-407`）。
- `RemoveError` 与 `Display` → `src/errors.rs`（`src/main.rs:291-338`）。
- `RemoveConfig` 与方法 → `src/config.rs`（`src/main.rs:340-378`）。
- `RemoveProgress` → `src/progress.rs`（`src/main.rs:41-137`）。
- `ProgressDisplay` → `src/progress.rs`（`src/main.rs:139-289`）。
- 结果处理与汇总 → `src/results.rs`（`src/main.rs:409-478`）。
- 路径去重/重叠检测 → `src/path.rs`（`src/main.rs:480-533`）。
- 删除逻辑：`fast_remove`/`remove_file`/`remove_symlink`/`remove_directory` → `src/removal.rs`（`src/main.rs:616-772`、`774-797`）。
- 测试：按阶段迁移到各模块或 `tests/`，保留安全检查（`src/main.rs:799-1098`）。

## 重构步骤
1. 新增各模块文件，剪切并粘贴对应代码片段；按需添加 `use` 导入与 `pub` 可见性。
2. 在 `main.rs` 顶部 `mod`/`use` 各模块公开 API，替换直接引用为模块路径引用。
3. 将共享类型（`Verbosity`）保留在 `config.rs` 或单独 `verbosity.rs`（推荐仍放 `config.rs`）。
4. 编译与修正可见性/导入错误；确保无行为变化（dry-run/TUI/并发/错误处理）。
5. 运行现有测试；将单元测试分散到各模块或迁移为 `tests/` 集成测试（不改断言）。

## 验证与测试
- 构建与运行：`cargo build`、`cargo run -- -n -v <tmpdir>` 验证 TUI 与 dry-run。
- 单元测试：保留逻辑测试（配置、路径去重、错误显示）。
- 集成测试：
  - dry-run：不触发删除；
  - 真实删除（`#[ignore]`）：需显式运行；
  - 权限错误场景：`continue_on_error` 覆盖。

## 兼容性与风险
- 依赖与行为不变；仅拆分为模块，风险集中在可见性与导入路径调整。
- TUI 线程共享状态依赖 `Arc<RemoveProgress>` 与 `AtomicBool`，保持原语义。
- 以最小改动完成迁移，避免异步/并发策略变化。

## 交付物
- 新增 7 个模块文件与精简 `main.rs`；测试按阶段整理。
- 保证编译通过与测试通过；用户界面与输出完全保持一致。