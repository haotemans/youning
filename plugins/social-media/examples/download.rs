//! Manual end-to-end download check, without sending to a group.
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicUsize, Ordering}}};
use yuantuan_plugin_api::*;
struct Host { root:PathBuf, next:AtomicUsize }
#[async_trait]
impl HostServices for Host {
    async fn chat(&self,_:&ToolCtx,_:&str,_:&str)->Result<String>{anyhow::bail!("not needed")}
    async fn read_data(&self,_:&str)->Result<Option<Value>>{Ok(None)}
    async fn write_data(&self,_:&str,_:Value)->Result<()>{Ok(())}
    fn log(&self,s:&str){eprintln!("{s}");}
    async fn artifact_path(&self,ext:&str)->Result<String>{
        Ok(self.root.join(format!("{}.{}",self.next.fetch_add(1,Ordering::Relaxed),ext)).to_string_lossy().into_owned())
    }
}
#[tokio::main]
async fn main()->Result<()> {
    let input=std::env::args().nth(1).ok_or_else(||anyhow::anyhow!("usage: download <share-url>"))?;
    let root=std::env::temp_dir().join(format!("yt-social-download-{}",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
    tokio::fs::create_dir_all(&root).await?;
    let p=yuantuan_plugin_social_media::plugin();
    let caps=p.start(PluginContext{host:Arc::new(Host{root:root.clone(),next:AtomicUsize::new(0)})},json!({})).await?;
    let ctx=ToolCtx{task_id:None,chat_id:"local-test".into(),chat_type:"private".into(),sender_pid:"local-test".into(),locale:None};
    let out=tokio::time::timeout(std::time::Duration::from_secs(120),caps.tools[0].call(&ctx,json!({"url":input}))).await??;
    println!("{}",json!({"files":out.artifacts,"bytes":out.data["total_bytes"]}));
    Ok(())
}
