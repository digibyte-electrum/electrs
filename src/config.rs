use bitcoin::p2p::Magic;
use bitcoin::Network;
use bitcoincore_rpc::Auth;
use dirs_next::home_dir;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::str::FromStr;

use std::env::consts::{ARCH, OS};
use std::time::Duration;

pub const ELECTRS_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_SERVER_ADDRESS: [u8; 4] = [127, 0, 0, 1]; // by default, serve on IPv4 localhost

mod internal {
    include!(concat!(env!("OUT_DIR"), "/configure_me_config.rs"));
}

/// A simple error type representing invalid UTF-8 input.
pub struct InvalidUtf8(OsString);

impl fmt::Display for InvalidUtf8 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?} isn't a valid UTF-8 sequence", self.0)
    }
}

/// An error that might happen when resolving an address
pub enum AddressError {
    ResolvError { addr: String, err: std::io::Error },
    NoAddrError(String),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AddressError::ResolvError { addr, err } => {
                write!(f, "Failed to resolve address {}: {}", addr, err)
            }
            AddressError::NoAddrError(addr) => write!(f, "No address found for {}", addr),
        }
    }
}

/// Newtype for an address that is parsed as `String`
///
/// The main point of this newtype is to provide better description than what `String` type
/// provides.
#[derive(Deserialize)]
pub struct ResolvAddr(String);

impl ::configure_me::parse_arg::ParseArg for ResolvAddr {
    type Error = InvalidUtf8;

    fn parse_arg(arg: &OsStr) -> std::result::Result<Self, Self::Error> {
        Self::parse_owned_arg(arg.to_owned())
    }

    fn parse_owned_arg(arg: OsString) -> std::result::Result<Self, Self::Error> {
        arg.into_string().map_err(InvalidUtf8).map(ResolvAddr)
    }

    fn describe_type<W: fmt::Write>(mut writer: W) -> fmt::Result {
        write!(writer, "a network address (will be resolved if needed)")
    }
}

impl ResolvAddr {
    /// Resolves the address.
    fn resolve(self) -> std::result::Result<SocketAddr, AddressError> {
        match self.0.to_socket_addrs() {
            Ok(iter) => select_resolved_addr(iter).ok_or(AddressError::NoAddrError(self.0)),
            Err(err) => Err(AddressError::ResolvError { addr: self.0, err }),
        }
    }

    /// Resolves the address, but prints error and exits in case of failure.
    fn resolve_or_exit(self) -> SocketAddr {
        self.resolve().unwrap_or_else(|err| {
            eprintln!("Error: {}", err);
            std::process::exit(1)
        })
    }
}

fn select_resolved_addr<I>(addresses: I) -> Option<SocketAddr>
where
    I: IntoIterator<Item = SocketAddr>,
{
    let mut fallback = None;
    for address in addresses {
        if address.is_ipv4() {
            return Some(address);
        }
        if fallback.is_none() {
            fallback = Some(address);
        }
    }
    fallback
}

/// This newtype implements `ParseArg` for `Network`.
#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum ElectrsNetwork {
    Bitcoin(Network),
    DigiByte,
}

impl Default for ElectrsNetwork {
    fn default() -> Self {
        ElectrsNetwork::Bitcoin(Network::Bitcoin)
    }
}

impl FromStr for ElectrsNetwork {
    type Err = String;

    fn from_str(string: &str) -> std::result::Result<Self, Self::Err> {
        if string.eq_ignore_ascii_case("digibyte")
            || string.eq_ignore_ascii_case("dgb")
        {
            // Temporary placeholder. We will give DigiByte its own
            // chain parameters in the next steps.
            return Ok(ElectrsNetwork::DigiByte);
        }

        Network::from_str(string)
            .map(ElectrsNetwork::Bitcoin)
            .map_err(|err| err.to_string())
    }
}

impl ::configure_me::parse_arg::ParseArgFromStr for ElectrsNetwork {
    fn describe_type<W: fmt::Write>(mut writer: W) -> fmt::Result {
        write!(
            writer,
            "either 'bitcoin', 'testnet', 'testnet4', 'regtest', 'signet' or 'digibyte'"
        )
    }
}

impl From<ElectrsNetwork> for Network {
    fn from(network: ElectrsNetwork) -> Network {
        match network {
            ElectrsNetwork::Bitcoin(network) => network,
            ElectrsNetwork::DigiByte => Network::Bitcoin,
        }
    }
}

/// Parsed and post-processed configuration
#[derive(Debug)]
pub struct Config {
    // See below for the documentation of each field:
    pub network: ElectrsNetwork,
    pub db_path: PathBuf,
    pub db_log_dir: Option<PathBuf>,
    pub db_parallelism: u8,
    pub daemon_auth: SensitiveAuth,
    pub daemon_rpc_addr: SocketAddr,
    pub daemon_p2p_addr: SocketAddr,
    pub electrum_rpc_addr: SocketAddr,
    pub monitoring_addr: SocketAddr,
    pub wait_duration: Duration,
    pub jsonrpc_timeout: Duration,
    pub index_batch_size: usize,
    pub index_lookup_limit: Option<usize>,
    pub reindex_last_blocks: usize,
    pub auto_reindex: bool,
    pub ignore_mempool: bool,
    pub sync_once: bool,
    pub skip_block_download_wait: bool,
    pub disable_electrum_rpc: bool,
    pub server_banner: String,
    pub signet_magic: Magic,
}

pub struct SensitiveAuth(pub Auth);

impl SensitiveAuth {
    pub(crate) fn get_auth(&self) -> Auth {
        self.0.clone()
    }
}

impl fmt::Debug for SensitiveAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Auth::UserPass(ref user, _) => f
                .debug_tuple("UserPass")
                .field(&user)
                .field(&"<sensitive>")
                .finish(),
            _ => write!(f, "{:?}", self.0),
        }
    }
}

/// Returns default daemon directory
fn default_daemon_dir() -> PathBuf {
    let mut home = home_dir().unwrap_or_else(|| {
        eprintln!("Error: unknown home directory");
        std::process::exit(1)
    });
    home.push(".bitcoin");
    home
}

fn default_config_files() -> Vec<OsString> {
    let mut files = vec![OsString::from("electrs.toml")]; // cwd
    if let Some(mut path) = home_dir() {
        path.extend([".electrs", "config.toml"]);
        files.push(OsString::from(path)) // home directory
    }
    files.push(OsString::from("/etc/electrs/config.toml")); // system-wide
    files
}

impl Config {
    /// Parses args, env vars, config files and post-processes them
    pub fn from_args() -> Config {
        use internal::prelude::ResultExt;

        let (mut config, _args) =
            internal::prelude::Config::including_optional_config_files(default_config_files())
                .unwrap_or_exit();

        fn unsupported_network(network: Network) -> ! {
            eprintln!("Error: unsupported network: {}", network);
            std::process::exit(1);
        }

        let db_subdir = match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => "bitcoin",
            ElectrsNetwork::Bitcoin(Network::Testnet) => "testnet",
            ElectrsNetwork::Bitcoin(Network::Testnet4) => "testnet4",
            ElectrsNetwork::Bitcoin(Network::Regtest) => "regtest",
            ElectrsNetwork::Bitcoin(Network::Signet) => "signet",
            ElectrsNetwork::DigiByte => "digibyte",
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        };

        config.db_dir.push(db_subdir);

        let default_daemon_rpc_port = match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => 8332,
            ElectrsNetwork::Bitcoin(Network::Testnet) => 18332,
            ElectrsNetwork::Bitcoin(Network::Testnet4) => 48332,
            ElectrsNetwork::Bitcoin(Network::Regtest) => 18443,
            ElectrsNetwork::Bitcoin(Network::Signet) => 38332,
            ElectrsNetwork::DigiByte => 14022,
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        };
        let default_daemon_p2p_port = match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => 8333,
            ElectrsNetwork::Bitcoin(Network::Testnet) => 18333,
            ElectrsNetwork::Bitcoin(Network::Testnet4) => 48333,
            ElectrsNetwork::Bitcoin(Network::Regtest) => 18444,
            ElectrsNetwork::Bitcoin(Network::Signet) => 38333,
            ElectrsNetwork::DigiByte => 12024,
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        };
        let default_electrum_port = match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => 50001,
            ElectrsNetwork::Bitcoin(Network::Testnet) => 60001,
            ElectrsNetwork::Bitcoin(Network::Testnet4) => 40001,
            ElectrsNetwork::Bitcoin(Network::Regtest) => 60401,
            ElectrsNetwork::Bitcoin(Network::Signet) => 60601,
            ElectrsNetwork::DigiByte => 50001,
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        };
        let default_monitoring_port = match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => 4224,
            ElectrsNetwork::Bitcoin(Network::Testnet) => 14224,
            ElectrsNetwork::Bitcoin(Network::Testnet4) => 44224,
            ElectrsNetwork::Bitcoin(Network::Regtest) => 24224,
            ElectrsNetwork::Bitcoin(Network::Signet) => 34224,
            ElectrsNetwork::DigiByte => 4225,
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        };

        let magic = match (config.network, config.signet_magic) {
            (ElectrsNetwork::Bitcoin(Network::Signet), Some(magic)) => {
                magic.parse().unwrap_or_else(|error| {
                    eprintln!(
                        "Error: signet magic '{}' is not a valid hex string: {}",
                        magic, error
                    );
                    std::process::exit(1);
                })
            }

            (ElectrsNetwork::DigiByte, None) => {
                Magic::from_bytes([0xfa, 0xc3, 0xb6, 0xda])
            }

            (ElectrsNetwork::Bitcoin(network), None) => network.magic(),

            (_, Some(_)) => {
                eprintln!("Error: signet magic only available on signet");
                std::process::exit(1);
            }
        };

        let daemon_rpc_addr: SocketAddr = config.daemon_rpc_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_daemon_rpc_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        let daemon_p2p_addr: SocketAddr = config.daemon_p2p_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_daemon_p2p_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        let electrum_rpc_addr: SocketAddr = config.electrum_rpc_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_electrum_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        #[cfg(not(feature = "metrics"))]
        {
            if config.monitoring_addr.is_some() {
                eprintln!("Error: enable \"metrics\" feature to specify monitoring_addr");
                std::process::exit(1);
            }
        }
        let monitoring_addr: SocketAddr = config.monitoring_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_monitoring_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        match config.network {
            ElectrsNetwork::Bitcoin(Network::Bitcoin) => (),
            ElectrsNetwork::Bitcoin(Network::Testnet) => config.daemon_dir.push("testnet3"),
            ElectrsNetwork::Bitcoin(Network::Testnet4) => config.daemon_dir.push("testnet4"),
            ElectrsNetwork::Bitcoin(Network::Regtest) => config.daemon_dir.push("regtest"),
            ElectrsNetwork::Bitcoin(Network::Signet) => config.daemon_dir.push("signet"),
            ElectrsNetwork::DigiByte => (),
            ElectrsNetwork::Bitcoin(unsupported) => unsupported_network(unsupported),
        }

        let mut deprecated_options_used = false;

        if config.timestamp {
            eprintln!(
                "Error: `timestamp` is deprecated, timestamps on logs is (and was) always \
                enabled, please remove this option."
            );
            deprecated_options_used = true;
        }

        if config.verbose > 0 {
            eprintln!("Error: please use `log_filters` to set logging verbosity",);
            deprecated_options_used = true;
        }

        if deprecated_options_used {
            std::process::exit(1);
        }

        let daemon_dir = &config.daemon_dir;
        let daemon_auth = SensitiveAuth(match (config.auth, config.cookie_file) {
            (None, None) => Auth::CookieFile(daemon_dir.join(".cookie")),
            (None, Some(cookie_file)) => Auth::CookieFile(cookie_file),
            (Some(auth), None) => {
                let parts: Vec<&str> = auth.splitn(2, ':').collect();
                if parts.len() != 2 {
                    eprintln!("Error: auth cookie doesn't contain colon");
                    std::process::exit(1);
                }
                Auth::UserPass(parts[0].to_owned(), parts[1].to_owned())
            }
            (Some(_), Some(_)) => {
                eprintln!("Error: ambiguous configuration - auth and cookie_file can't be specified at the same time");
                std::process::exit(1);
            }
        });

        let log_filters = config.log_filters;

        let index_lookup_limit = match config.index_lookup_limit {
            0 => None,
            _ => Some(config.index_lookup_limit),
        };

        if config.jsonrpc_timeout_secs <= config.wait_duration_secs {
            eprintln!(
                "Error: jsonrpc_timeout_secs ({}) must be higher than wait_duration_secs ({})",
                config.jsonrpc_timeout_secs, config.wait_duration_secs
            );
            std::process::exit(1);
        }

        if config.version {
            println!("v{}", ELECTRS_VERSION);
            std::process::exit(0);
        }

        let config = Config {
            network: config.network,
            db_path: config.db_dir,
            db_log_dir: config.db_log_dir,
            db_parallelism: config.db_parallelism,
            daemon_auth,
            daemon_rpc_addr,
            daemon_p2p_addr,
            electrum_rpc_addr,
            monitoring_addr,
            wait_duration: Duration::from_secs(config.wait_duration_secs),
            jsonrpc_timeout: Duration::from_secs(config.jsonrpc_timeout_secs),
            index_batch_size: config.index_batch_size,
            index_lookup_limit,
            reindex_last_blocks: config.reindex_last_blocks,
            auto_reindex: config.auto_reindex,
            ignore_mempool: config.ignore_mempool,
            sync_once: config.sync_once,
            skip_block_download_wait: config.skip_block_download_wait,
            disable_electrum_rpc: config.disable_electrum_rpc,
            server_banner: config.server_banner,
            signet_magic: magic,
        };
        eprintln!(
            "Starting electrs {} on {} {} with {:?}",
            ELECTRS_VERSION, ARCH, OS, config
        );
        let mut builder = env_logger::Builder::from_default_env();
        builder.default_format().format_timestamp_millis();
        if let Some(log_filters) = &log_filters {
            builder.parse_filters(log_filters);
        }
        builder.init();

        config
    }
}

#[cfg(test)]
mod tests {
    use super::{select_resolved_addr, Auth, SensitiveAuth};
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::path::Path;

    #[test]
    fn test_resolved_address_prefers_ipv4_with_ipv6_fallback() {
        let ipv4 = SocketAddr::from((Ipv4Addr::LOCALHOST, 14022));
        let ipv6 = SocketAddr::from((Ipv6Addr::LOCALHOST, 14022));

        assert_eq!(select_resolved_addr([ipv6, ipv4]), Some(ipv4));
        assert_eq!(select_resolved_addr([ipv6]), Some(ipv6));
    }

    #[test]
    fn test_auth_debug() {
        let auth = Auth::None;
        assert_eq!(format!("{:?}", SensitiveAuth(auth)), "None");

        let auth = Auth::CookieFile(Path::new("/foo/bar/.cookie").to_path_buf());
        assert_eq!(
            format!("{:?}", SensitiveAuth(auth)),
            "CookieFile(\"/foo/bar/.cookie\")"
        );

        let auth = Auth::UserPass("user".to_owned(), "pass".to_owned());
        assert_eq!(
            format!("{:?}", SensitiveAuth(auth)),
            "UserPass(\"user\", \"<sensitive>\")"
        );
    }
}
