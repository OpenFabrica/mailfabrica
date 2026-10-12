//! 所有发布产物使用共享的原创代码图标。
use std::path::Path;
/// 将共享的透明 PNG 写入打包目录，报告完整的写入错误。
pub(crate) fn render(size: u32, output: &Path) -> Result<(), String> {
    std::fs::write(output, mailfabrica_brand::png(size)?).map_err(|e| format!("{}: {e}", output.display()))
}
