use clap::Parser;

/// Anthropic <-> Kiro API 客户端
#[derive(Parser, Debug)]
#[command(about, long_about = None)]
#[command(version = env!("GIT_VERSION"))]
pub struct Args {
    /// 配置文件路径
    #[arg(short, long)]
    pub config: Option<String>,

    /// 凭证文件路径
    #[arg(long)]
    pub credentials: Option<String>,
}
