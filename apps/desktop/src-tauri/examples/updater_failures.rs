//! Hosted tests only. Tauri supplies mock application scaffolding; updater I/O is real.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri_plugin_updater::{Error, UpdaterExt};

fn require_host() -> Result<(), String> {
    for (key, expected) in [
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_EVENT_NAME", "workflow_dispatch"),
        ("RUNNER_ENVIRONMENT", "github-hosted"),
        ("RUNNER_OS", "macOS"),
        ("RUNNER_ARCH", "ARM64"),
        ("XTRACE_TELEMETRY", "0"),
    ] {
        if std::env::var(key).as_deref() != Ok(expected) {
            return Err(format!("Hosted helper requires {key}={expected}"));
        }
    }
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) || unsafe { libc::geteuid() } == 0 {
        return Err("Requires a non-root hosted macOS arm64 process".into());
    }
    let os = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|e| e.to_string())?;
    if !os.status.success() || !os.stdout.starts_with(b"14.") {
        return Err("Requires macOS 14".into());
    }
    Ok(())
}

fn text<'a>(config: &'a Value, key: &str) -> Result<&'a str, String> {
    config[key].as_str().ok_or_else(|| format!("Missing {key}"))
}

/// The release pair and archive size the harness pins in one place
/// (scripts/ci/hosted-release.mjs); this helper only checks their shape.
struct Pins {
    old: String,
    target: String,
    mismatch: String,
    archive_bytes: usize,
}

fn pins(config: &Value) -> Result<Pins, String> {
    let versions = &config["versions"];
    let version = |key: &str| -> Result<String, String> {
        let value = versions[key]
            .as_str()
            .ok_or_else(|| format!("Missing version {key}"))?;
        let parts: Vec<_> = value.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(format!("Version {key} is not MAJOR.MINOR.PATCH"));
        }
        Ok(value.to_string())
    };
    let (old, target, mismatch) = (version("old")?, version("target")?, version("mismatch")?);
    if old == target || target == mismatch || old == mismatch {
        return Err("The old, target and mismatch versions must differ".into());
    }
    let archive_bytes = config["archiveBytes"]
        .as_u64()
        .filter(|bytes| *bytes > 0)
        .ok_or("Missing archive size")? as usize;
    Ok(Pins {
        old,
        target,
        mismatch,
        archive_bytes,
    })
}

fn process_identity() -> Result<Value, String> {
    #[cfg(target_os = "macos")]
    {
        // Kernel birth time binds every receipt to this process, not a reused PID.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        let read = unsafe {
            libc::proc_pidinfo(
                std::process::id() as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if read != size || info.pbi_pid != std::process::id() || info.pbi_start_tvsec == 0 {
            return Err("Cannot obtain own kernel process identity".into());
        }
        Ok(json!({"pid":info.pbi_pid, "uid":info.pbi_uid,
            "startMicros":info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
            "executable":std::env::current_exe().map_err(|e| e.to_string())?}))
    }
    #[cfg(not(target_os = "macos"))]
    Err("Requires macOS process identity".into())
}

fn owned(root: &Path, value: &str) -> Result<PathBuf, String> {
    let path = fs::canonicalize(value).map_err(|e| e.to_string())?;
    if !path.starts_with(root) || path == root || path != Path::new(value) {
        return Err("Path is not canonical and strictly inside owned staging".into());
    }
    Ok(path)
}

fn execute(config: &Value) -> Result<Value, String> {
    require_host()?;
    let process = process_identity()?;
    let root = fs::canonicalize(text(config, "root")?).map_err(|e| e.to_string())?;
    if root != Path::new(text(config, "root")?)
        || root.file_name().and_then(|s| s.to_str()) != Some("staging.noindex")
    {
        return Err("Invalid owned staging root".into());
    }
    let identifier = text(config, "identifier")?;
    if !identifier.starts_with("ai.xtrace.app.test.updater-failures.")
        || !identifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("Invalid test identifier".into());
    }
    let executable = owned(&root, text(config, "targetExecutable")?)?;
    let target = owned(&root, text(config, "target")?)?;
    if tauri_plugin_updater::extract_path_from_executable(&executable).map_err(|e| e.to_string())?
        != target
    {
        return Err("SDK target derivation differs from owned bundle".into());
    }
    for key in [
        "HOME",
        "TMPDIR",
        "XTRACE_DATA_DIR",
        "XTRACE_NATIVE_HOME",
        "CLAUDE_CONFIG_DIR",
        "GH_CONFIG_DIR",
    ] {
        owned(&root, &std::env::var(key).map_err(|e| e.to_string())?)?;
    }
    let case = text(config, "case")?;
    let pins = pins(config)?;
    if case == "probe" {
        if fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            != executable
        {
            return Err("Old-app probe is not running from the owned target".into());
        }
        let sentinel = fs::read_to_string(
            Path::new(&std::env::var("XTRACE_DATA_DIR").unwrap()).join("sentinel.txt"),
        )
        .map_err(|e| e.to_string())?;
        if sentinel != text(config, "sentinel")? {
            return Err("Old-app probe could not read its synthetic sentinel".into());
        }
        return Ok(
            json!({"status":"passed", "case":case, "identifier":identifier,
                         "pid":std::process::id(), "process":process, "oldTargetUsable":true}),
        );
    }
    if ![
        "valid",
        "corrupt",
        "version",
        "timeout",
        "interrupted",
        "permission",
    ]
    .contains(&case)
    {
        return Err("Unknown case".into());
    }
    let endpoint: url::Url = text(config, "endpoint")?
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;
    if endpoint.scheme() != "http"
        || endpoint.host_str() != Some("127.0.0.1")
        || endpoint.port().is_none()
        || endpoint.path() != format!("/{case}/latest.json")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err("Endpoint must be the owned loopback case".into());
    }
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = identifier.into();
    context.package_info_mut().version = pins.old.parse().map_err(|e| format!("{e}"))?;
    context.config_mut().plugins.0.insert(
        "updater".into(),
        json!({"pubkey": text(config, "pubkey")?, "endpoints":[endpoint],
               "requireSignedVersion":true, "dangerousInsecureTransportProtocol":true}),
    );
    // Do not run this mock runtime. Its main-thread dispatch executes immediately,
    // so the macOS install call below executes the real osakit closure on main.
    let app = mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .map_err(|e| e.to_string())?;
    let updater = app
        .updater_builder()
        .executable_path(&executable)
        .target("darwin-aarch64")
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let mut update = tauri::async_runtime::block_on(updater.check())
        .map_err(|e| e.to_string())?
        .ok_or("SDK returned no update")?;
    let expected_download = endpoint.join("archive").map_err(|e| e.to_string())?;
    if update.download_url != expected_download
        || update.signature != text(config, "signature")?
        || update.version
            != if case == "version" {
                pins.mismatch.as_str()
            } else {
                pins.target.as_str()
            }
    {
        return Err("SDK response differs from the owned fixture".into());
    }
    // check() deliberately does not carry the builder timeout into Update.
    update.timeout = Some(Duration::from_secs(600));
    let mut transferred = 0_usize;
    let mut finished = false;
    let start = Instant::now();
    let download = tauri::async_runtime::block_on(
        update.download(|n, _| transferred += n, || finished = true),
    );
    let elapsed = start.elapsed().as_millis() as u64;
    let mut result = json!({"case":case, "pid":std::process::id(), "process":process,
        "transferred":transferred, "transferFinishedCallback":finished,
        "elapsedMs":elapsed, "downloadTimeoutMs":600000,
        "verifiedDownload":false, "installAttempted":false, "status":"failed"});
    match download {
        Err(error) => {
            result["error"] = json!(error.to_string());
            result["errorDebug"] = json!(format!("{error:?}"));
            let expected = match (case, &error) {
                ("corrupt", Error::Minisign(_)) => true,
                ("version", Error::SignedVersionMismatch { signed, announced }) => {
                    *signed == pins.target && *announced == pins.mismatch
                }
                ("timeout", Error::Reqwest(e)) => {
                    e.is_timeout() && (599_000..=630_000).contains(&elapsed) && transferred > 0
                }
                ("interrupted", Error::Reqwest(e)) => !e.is_timeout() && transferred > 0,
                _ => false,
            };
            if expected {
                result["status"] = json!("passed");
            }
        }
        Ok(bytes) => {
            result["verifiedDownload"] = json!(true);
            result["verifiedBytes"] = json!(bytes.len());
            if !["valid", "permission"].contains(&case) {
                result["error"] = json!("A rejection case unexpectedly returned verified bytes");
                return Ok(result);
            }
            if bytes.len() != pins.archive_bytes {
                return Err("Verified archive length differs from pinned release".into());
            }
            // Only verified SDK download output can enter installation.
            result["installAttempted"] = json!(true);
            let receipt = owned(&root, text(config, "installReceipt")?)?;
            fs::write(receipt, result.to_string()).map_err(|e| e.to_string())?;
            let install_start = Instant::now();
            match update.install(&bytes) {
                Ok(()) if case == "valid" => result["status"] = json!("passed"),
                Ok(()) => result["error"] = json!("Protected installation unexpectedly succeeded"),
                Err(error) => {
                    result["error"] = json!(error.to_string());
                    result["errorDebug"] = json!(format!("{error:?}"));
                    if case == "permission"
                        && matches!(&error, Error::Io(e)
                            if e.kind() == std::io::ErrorKind::PermissionDenied
                            && e.to_string() == "Failed to move the new app into place")
                    {
                        // Parent also requires the independent real Cancel receipt.
                        result["status"] = json!("passed");
                    }
                }
            }
            result["installElapsedMs"] = json!(install_start.elapsed().as_millis() as u64);
        }
    }
    Ok(result)
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let outcome = (|| {
        if args.len() != 2 {
            return Err("Expected one owned configuration path".into());
        }
        let config = serde_json::from_slice(&fs::read(&args[1]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        execute(&config)
    })();
    let result = outcome.unwrap_or_else(|error: String| json!({"status":"failed", "error":error}));
    println!("{result}");
    if result["status"] != "passed" {
        std::process::exit(1);
    }
}
