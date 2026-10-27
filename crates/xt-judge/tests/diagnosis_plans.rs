//! Pure Diagnose plans and discovery with synthetic homes and executable files.
#![cfg(unix)]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use xt_judge::diagnosis::agents::{self, Limit, Tool};
use xt_probes::executable::Unavailable;

fn fake_tool(path: &Path) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    path.to_owned()
}

#[test]
fn tools_are_found_from_overrides_path_and_known_folders() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let bin = root.path().join("bin");
    let claude = fake_tool(&bin.join("claude"));
    let codex = fake_tool(&bin.join("codex"));
    let cursor = fake_tool(&home.join(".local/bin/cursor-agent"));
    let env = vec![
        ("HOME".into(), home.clone().into()),
        ("PATH".into(), bin.into()),
        (agents::CODEX_ENV.into(), codex.clone().into()),
    ];
    let found = agents::find_tools(&env);
    assert_eq!(found.claude, Ok(claude.clone()));
    assert_eq!(found.codex, Ok(codex.clone()));
    assert_eq!(found.cursor, Ok(cursor));
    let env: Vec<(OsString, OsString)> = vec![
        ("HOME".into(), home.into()),
        ("PATH".into(), "/nonexistent".into()),
        (xt_probes::executable::CLAUDE_ENV.into(), claude.into()),
        (agents::CODEX_ENV.into(), codex.into()),
        (agents::CURSOR_ENV.into(), "relative/cursor".into()),
    ];
    assert_eq!(
        agents::find_tools(&env).cursor,
        Err(Unavailable::OverrideNotAbsolute)
    );
}

#[test]
fn environment_keeps_only_allowed_names_and_values_for_each_tool() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let codex_home = home.join(".codex");
    let binary = root.path().join("tool-bin/agent");
    let inherited_bin = root.path().join("inherited-bin");
    let path = std::env::join_paths([
        Path::new("relative/bin"),
        Path::new("."),
        Path::new(""),
        inherited_bin.as_path(),
    ])
    .unwrap();
    let inherited = vec![
        ("HOME".into(), home.clone().into()),
        ("USER".into(), "synthetic-user".into()),
        ("LOGNAME".into(), "synthetic-logname".into()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("TMPDIR".into(), root.path().join("tmp").into()),
        ("LC_ALL".into(), "C".into()),
        ("LC_TIME".into(), "en_GB.UTF-8".into()),
        ("PATH".into(), path),
        ("CODEX_HOME".into(), codex_home.clone().into()),
        ("ANTHROPIC_API_KEY".into(), "synthetic-anthropic-key".into()),
        ("OPENAI_API_KEY".into(), "synthetic-openai-key".into()),
        ("CURSOR_API_KEY".into(), "synthetic-cursor-key".into()),
        ("CLAUDECODE".into(), "1".into()),
        ("CLAUDE_CODE_USE_BEDROCK".into(), "1".into()),
        ("DISABLE_AUTOUPDATER".into(), "0".into()),
        ("NO_COLOR".into(), "0".into()),
    ];
    for tool in [Tool::ClaudeCode, Tool::Codex, Tool::Cursor] {
        let env = agents::agent_env(tool, &binary, &inherited);
        let values: BTreeMap<_, _> = env.iter().cloned().collect();
        assert_eq!(values.len(), env.len(), "duplicate names for {tool:?}");
        let expected_names = [
            "HOME",
            "LANG",
            "LC_ALL",
            "LC_TIME",
            "LOGNAME",
            "USER",
            "TMPDIR",
            "PATH",
            "DISABLE_AUTOUPDATER",
            "NO_COLOR",
        ];
        assert_eq!(
            values.len(),
            expected_names.len() + usize::from(tool == Tool::Codex)
        );
        for name in expected_names {
            assert!(
                values.contains_key(&OsString::from(name)),
                "{tool:?}: {name}"
            );
        }
        for name in [
            "HOME", "USER", "LOGNAME", "LANG", "TMPDIR", "LC_ALL", "LC_TIME",
        ] {
            assert_eq!(
                values.get(&OsString::from(name)),
                inherited
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value),
                "{tool:?}: {name}"
            );
        }
        assert_eq!(
            values.get(&OsString::from("CODEX_HOME")),
            (tool == Tool::Codex).then_some(&codex_home.clone().into_os_string())
        );
        assert_eq!(values[&OsString::from("DISABLE_AUTOUPDATER")], "1");
        assert_eq!(values[&OsString::from("NO_COLOR")], "1");
        for forbidden in [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "CURSOR_API_KEY",
            "CLAUDECODE",
            "CLAUDE_CODE_USE_BEDROCK",
        ] {
            assert!(
                !values.contains_key(&OsString::from(forbidden)),
                "{tool:?}: {forbidden}"
            );
        }
        let entries: Vec<_> = std::env::split_paths(&values[&OsString::from("PATH")]).collect();
        assert_eq!(entries[0], binary.parent().unwrap());
        assert_eq!(entries[1], inherited_bin);
        assert!(entries.iter().all(|entry| entry.is_absolute()));
        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));
    }
}

#[test]
fn default_order_excludes_cursor_and_keeps_tool_limits_explicit() {
    assert_eq!(agents::ORDER, [Tool::ClaudeCode, Tool::Codex]);
    assert!(Tool::ClaudeCode.limits().is_empty());
    assert_eq!(Tool::Codex.limits(), [Limit::CodexLoadsUserInstructions]);
    assert_eq!(
        Tool::Cursor.limits(),
        [Limit::CursorPromptOnCommandLine, Limit::CursorUnverified]
    );
}
