//! `tokenloom` CLI entrypoint (PLAN.md §8).

mod cache;
mod commands;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokenloom_core::Config;

/// Fast, safe, token-efficient web search & page fetching for LLMs.
#[derive(Parser, Debug)]
#[command(name = "tokenloom", version, about)]
struct Cli {
    /// Path to a TOML config file (highest precedence after flags).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Verbose diagnostics to stderr (-v info, -vv debug, -vvv trace).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Federated search across SearXNG-compatible engines (Markdown by default)
    Search {
        /// Query text; supports bangs like `!ddg`, `!arx`, `!news`
        query: String,
        /// Category: general|images|videos|news|map|music|it|science|files|social_media
        #[arg(long, short = 'c')]
        category: Option<String>,
        /// Comma-separated engine names to query explicitly
        #[arg(long, short = 'e')]
        engines: Option<String>,
        /// Max results returned
        #[arg(long, short = 'l')]
        limit: Option<usize>,
        #[arg(long, default_value = "1")]
        page: u32,
        #[arg(long)]
        locale: Option<String>,
        /// Filter: day | week | month | year
        #[arg(long)]
        time_range: Option<String>,
        /// 0 = off, 1 = moderate, 2 = strict
        #[arg(long, default_value = "1")]
        safe_search: u8,
        #[arg(long)]
        timeout_ms: Option<u64>,
        /// Stable JSON v1 output for agents
        #[arg(long)]
        json: bool,
        /// Truncate output to approximately this many tokens
        #[arg(long)]
        max_tokens: Option<usize>,
        /// Override the per-query engine cap (default 12)
        #[arg(long)]
        max_engines: Option<usize>,
    },
    /// Fetch a URL and convert it to clean Markdown
    #[command(alias = "read")]
    Fetch {
        url: String,
        /// Truncate the Markdown to approximately this many tokens
        #[arg(long)]
        max_tokens: Option<usize>,
        /// Truncate the Markdown to this many characters
        #[arg(long)]
        max_chars: Option<usize>,
        /// Skip the page cache (read & write)
        #[arg(long)]
        no_cache: bool,
        /// Disable SPA detection & Jina Reader delegation
        #[arg(long)]
        no_reader: bool,
        /// Include images in the Markdown output
        #[arg(long)]
        allow_images: bool,
        /// Queue with backoff when Jina is rate limited
        #[arg(long)]
        wait: bool,
        /// Stable JSON v1 output for agents
        #[arg(long)]
        json: bool,
    },
    /// Inspect & manage the 248-engine registry
    #[command(subcommand)]
    Engines(EnginesCommands),
    /// List or search bang shortcuts
    Bangs {
        /// Optional substring filter (e.g. "ddg" or "!sci")
        pattern: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Self-test DNS, SSRF guard, engines, Jina quota & headless browser
    Doctor,
    /// Inspect configuration
    #[command(subcommand)]
    Config(ConfigCommands),
    /// Check GitHub Releases for a newer version and self-update
    Update {
        /// Only report the available version; don't download or install
        #[arg(long)]
        check: bool,
        /// Update to a specific version (e.g. 0.1.7) instead of the latest
        #[arg(long)]
        to: Option<String>,
        /// Reinstall even when already up to date
        #[arg(long)]
        force: bool,
    },
    /// Launch the MCP stdio tool server (search & fetch tools)
    Mcp,
}

#[derive(Subcommand, Debug)]
enum EnginesCommands {
    /// List all configured engines with status & categories
    List {
        #[arg(long, short = 'c')]
        category: Option<String>,
        /// Only show engines that have a working implementation
        #[arg(long)]
        implemented_only: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show detailed info for one engine
    Show { name: String },
    /// Live-connectivity & parsing test for one engine
    Test { name: String, query: Option<String> },
    /// Enable an engine in the user config
    Enable { name: String },
    /// Disable an engine in the user config
    Disable { name: String },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Show the active config file path
    Path,
    /// Read the whole config or one dotted key (e.g. http.proxy)
    Get { key: Option<String> },
}

fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // html5ever's serializer emits benign `weird namespace` warnings when
    // readability round-trips fragments; keep stderr clean unless debugging.
    let level = format!("{level},html5ever=off");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level)),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let config = match Config::load(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tokenloom: config error: {e}");
            std::process::exit(2);
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let code = rt.block_on(async move {
        match cli.command {
            Commands::Search {
                query,
                category,
                engines,
                limit,
                page,
                locale,
                time_range,
                safe_search,
                timeout_ms,
                json,
                max_tokens,
                max_engines,
            } => {
                commands::search::run(
                    &config,
                    query,
                    category,
                    engines,
                    limit,
                    page,
                    locale,
                    time_range,
                    safe_search,
                    timeout_ms,
                    json,
                    max_tokens,
                    max_engines,
                )
                .await
            }
            Commands::Fetch {
                url,
                max_tokens,
                max_chars,
                no_cache,
                no_reader,
                allow_images,
                wait,
                json,
            } => {
                commands::fetch::run(
                    &config,
                    url,
                    max_tokens,
                    max_chars,
                    no_cache,
                    no_reader,
                    allow_images,
                    wait,
                    json,
                )
                .await
            }
            Commands::Engines(cmd) => commands::engines::run(&config, cmd).await,
            Commands::Bangs { pattern, json } => commands::bangs::run(&config, pattern, json),
            Commands::Doctor => commands::doctor::run(&config).await,
            Commands::Config(cmd) => commands::config_cmd::run(&config, cmd),
            Commands::Update { check, to, force } => {
                commands::update::run(&config, check, to, force).await
            }
            Commands::Mcp => commands::mcp::run(&config).await,
        }
    });

    if code != 0 {
        std::process::exit(code);
    }
}
