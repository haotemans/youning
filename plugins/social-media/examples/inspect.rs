#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: inspect <share-url>"))?;
    println!(
        "{}",
        yuantuan_plugin_social_media::inspect_link(&input).await?
    );
    Ok(())
}
