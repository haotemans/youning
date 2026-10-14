//! Tools 系统（架构文档十五章扩展模型第 2 条："能力扩展 = 新 Tool"）。
//!
//! Tool = 可被 Agent / 命令直派调用的最小能力单元；Registry 持有全部已注册 Tool。
//!
//! Agent 通过相同注册表读取工具描述、参数 schema 并执行工具。

use std::collections::HashMap;
use std::sync::Arc;

pub mod media;
pub mod sandbox;

pub use yuantuan_plugin_api::{Tool, ToolCtx, ToolOutput};

/// 工具注册表：进程内单例（Arc 共享），启动时把所有 Tool 注册进来。
/// 新增 Tool 的途径 = 实现 Tool + 在装配处 `registry.register(...)`。
#[derive(Default, Clone)]
pub struct Registry {
    inner: Arc<std::sync::RwLock<HashMap<&'static str, Arc<dyn Tool>>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&self, tool: T) {
        self.register_arc(Arc::new(tool));
    }

    /// 已 Arc 包装的工具直接注册（插件 register() 通常返回 Vec<Arc<dyn Tool>>）
    pub fn register_arc(&self, tool: Arc<dyn Tool>) {
        let name = tool.name();
        self.inner
            .write()
            .expect("tools registry poisoned")
            .insert(name, tool);
    }

    pub fn register_batch(&self, items: Vec<Arc<dyn Tool>>) -> anyhow::Result<()> {
        let mut registry = self.inner.write().unwrap();
        let mut names = std::collections::HashSet::new();
        for tool in &items {
            let name = tool.name();
            if registry.contains_key(name) || !names.insert(name) {
                anyhow::bail!("能力名称冲突：{name}");
            }
        }
        for tool in items {
            registry.insert(tool.name(), tool);
        }
        Ok(())
    }

    pub fn unregister(&self, names: &[&str]) {
        let mut registry = self.inner.write().unwrap();
        for name in names {
            registry.remove(*name);
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.inner
            .read()
            .expect("tools registry poisoned")
            .get(name)
            .cloned()
    }

    /// 列出所有已注册工具名（WebUI 调试页 / 命令帮助用）
    pub fn names(&self) -> Vec<&'static str> {
        self.inner
            .read()
            .expect("tools registry poisoned")
            .keys()
            .copied()
            .collect()
    }
}
