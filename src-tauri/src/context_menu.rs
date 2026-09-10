#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use std::fs;
#[cfg(target_os = "windows")]
use crate::cli::{self, CliKind};

/// Register system context menu for "Open with Claude Code".
pub fn register(skip_permissions: bool, terminal_app: Option<&str>) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        register_macos(skip_permissions, terminal_app)
    }

    #[cfg(target_os = "windows")]
    {
        let _ = terminal_app;
        register_windows(skip_permissions)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (skip_permissions, terminal_app);
        Err("不支持的平台".to_string())
    }
}

/// Unregister system context menu.
pub fn unregister() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        unregister_macos()
    }

    #[cfg(target_os = "windows")]
    {
        unregister_windows()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("不支持的平台".to_string())
    }
}

/// Check if system context menu is registered.
pub fn is_registered() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        is_registered_macos()
    }

    #[cfg(target_os = "windows")]
    {
        is_registered_windows()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
fn register_macos(skip_permissions: bool, terminal_app: Option<&str>) -> Result<String, String> {
    let home = dirs::home_dir().ok_or("无法获取用户目录")?;
    let services_dir = home.join("Library").join("Services");
    let workflow_dir = services_dir.join("Claude Code.workflow");
    let contents_dir = workflow_dir.join("Contents");

    // Clean up old install if exists
    if workflow_dir.exists() {
        let _ = fs::remove_dir_all(&workflow_dir);
    }

    fs::create_dir_all(&contents_dir).map_err(|e| format!("注册失败，权限不足。{}", e))?;

    // Info.plist — register as a service that receives folders
    let info_plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.sessiondock.claude-code-service</string>
    <key>CFBundleName</key>
    <string>Claude Code</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>Claude Code</string>
            </dict>
            <key>NSMessage</key>
            <string>runWorkflowAsService</string>
            <key>NSSendFileTypes</key>
            <array>
                <string>public.folder</string>
            </array>
        </dict>
    </array>
</dict>
</plist>"#;
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("注册失败，权限不足。{}", e))?;

    // Determine preferred terminal: 用户设置优先，未安装/未设置时回退自动探测
    let terminal_app = crate::terminal::macos_app_name(terminal_app);

    // Build claude command based on skip_permissions setting
    let claude_cmd = if skip_permissions {
        "claude --dangerously-skip-permissions"
    } else {
        "claude"
    };

    // document.wflow — Automator Quick Action that runs a shell script
    let document_wflow = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMCategory</key>
				<string>AMCategoryUtilities</string>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>for f in "$@"; do
    if [ -d "$f" ]; then
        SCRIPT=$(mktemp /tmp/sessiondock_ctx_XXXXXX.sh)
        echo '#!/bin/bash -l' &gt; "$SCRIPT"
        echo "cd '$f' &amp;&amp; {claude_cmd}" &gt;&gt; "$SCRIPT"
        chmod +x "$SCRIPT"
        open -a "{terminal_app}" "$SCRIPT"
        break
    fi
done</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/bash</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>A12BFEE3-6E2E-4D8E-AD72-E0F3C2D7A1B0</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
					<string>Command</string>
					<string>Run</string>
				</array>
				<key>OutputUUID</key>
				<string>B23CFFE4-7F3F-5E9F-BE83-F1A4D3E8B2C1</string>
				<key>UUID</key>
				<string>C34D00F5-8A40-6FA0-CF94-02B5E4F9C3D2</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject.folder</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>"#, terminal_app = terminal_app, claude_cmd = claude_cmd);

    fs::write(contents_dir.join("document.wflow"), document_wflow)
        .map_err(|e| format!("注册失败，权限不足。{}", e))?;

    // Refresh services cache
    std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-flush")
        .output()
        .ok();

    Ok("右键菜单注册成功。请在 Finder 中右键文件夹，在「快速操作」或「服务」子菜单中查找 Claude Code。".to_string())
}

#[cfg(target_os = "macos")]
fn unregister_macos() -> Result<String, String> {
    let home = dirs::home_dir().ok_or("无法获取用户目录")?;
    let workflow_dir = home
        .join("Library")
        .join("Services")
        .join("Claude Code.workflow");

    if workflow_dir.exists() {
        fs::remove_dir_all(&workflow_dir).map_err(|e| format!("取消注册失败。{}", e))?;
    }

    // Refresh services cache
    std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-flush")
        .output()
        .ok();

    Ok("右键菜单已取消注册。".to_string())
}

#[cfg(target_os = "macos")]
fn is_registered_macos() -> Result<bool, String> {
    let home = dirs::home_dir().ok_or("无法获取用户目录")?;
    let workflow_dir = home
        .join("Library")
        .join("Services")
        .join("Claude Code.workflow");
    Ok(workflow_dir.exists())
}

#[cfg(target_os = "windows")]
fn register_windows(skip_permissions: bool) -> Result<String, String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let cli_path = cli::find_cli_path(CliKind::Claude)
        .ok_or("未检测到 Claude Code CLI。请先安装 CLI，再注册 Windows 右键菜单。")?;
    let helper_script = write_windows_context_helper(skip_permissions, &cli_path)?;
    let helper_script_str = helper_script
        .to_str()
        .ok_or("右键菜单脚本路径含非 UTF-8 字符")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    write_windows_context_entry(
        &hkcu,
        r"Software\Classes\Directory\Background\shell\ClaudeCode",
        "Open with Claude Code",
        &cli_path,
        &windows_registry_command(helper_script_str, "%V"),
    )?;
    write_windows_context_entry(
        &hkcu,
        r"Software\Classes\Directory\shell\ClaudeCode",
        "Open with Claude Code",
        &cli_path,
        &windows_registry_command(helper_script_str, "%1"),
    )?;

    Ok("右键菜单注册成功。现在可在资源管理器的“文件夹空白处右键”与“文件夹项右键”中启动 Claude Code；Windows 下默认使用 cmd 新窗口，不依赖 Windows Terminal。".to_string())
}

#[cfg(target_os = "windows")]
fn unregister_windows() -> Result<String, String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Directory\Background\shell\ClaudeCode");
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Directory\shell\ClaudeCode");

    if let Ok(script_path) = windows_context_helper_path() {
        if let Err(e) = fs::remove_file(&script_path) {
            eprintln!("Failed to remove context menu helper script {:?}: {}", script_path, e);
        }
    }

    Ok("右键菜单已取消注册。".to_string())
}

#[cfg(target_os = "windows")]
fn is_registered_windows() -> Result<bool, String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // If the key can be opened, it means it's registered
    let key_exists = hkcu.open_subkey(r"Software\Classes\Directory\Background\shell\ClaudeCode").is_ok();
    Ok(key_exists)
}

#[cfg(target_os = "windows")]
fn write_windows_context_entry(
    hkcu: &winreg::RegKey,
    path: &str,
    title: &str,
    icon: &str,
    command: &str,
) -> Result<(), String> {
    let (key, _) = hkcu
        .create_subkey(path)
        .map_err(|e| format!("注册失败，权限不足。{}", e))?;
    key.set_value("", &title)
        .map_err(|e| format!("注册失败。{}", e))?;
    key.set_value("Icon", &icon)
        .map_err(|e| format!("注册失败。{}", e))?;

    let (cmd_key, _) = hkcu
        .create_subkey(&format!("{}\\command", path))
        .map_err(|e| format!("注册失败，权限不足。{}", e))?;
    cmd_key
        .set_value("", &command)
        .map_err(|e| format!("注册失败。{}", e))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_context_helper_path() -> Result<std::path::PathBuf, String> {
    let data_dir = dirs::data_dir().ok_or("无法获取应用数据目录")?;
    let dir = data_dir.join("com.sessiondock.app").join("config");
    fs::create_dir_all(&dir).map_err(|e| format!("创建右键菜单脚本目录失败: {}", e))?;
    Ok(dir.join("context-menu-claude.cmd"))
}

#[cfg(target_os = "windows")]
fn write_windows_context_helper(skip_permissions: bool, cli_path: &str) -> Result<std::path::PathBuf, String> {
    let path = windows_context_helper_path()?;
    let script = cli::build_windows_context_menu_script(CliKind::Claude, cli_path, skip_permissions);
    fs::write(&path, script).map_err(|e| format!("写入右键菜单脚本失败: {}", e))?;
    Ok(path)
}

#[cfg(target_os = "windows")]
fn windows_registry_command(script_path: &str, target_placeholder: &str) -> String {
    let escaped_script = script_path.replace('"', "\"\"");
    format!(
        r#"cmd.exe /d /c start "" "%COMSPEC%" /k "\"{}\" \"{}\"""#,
        escaped_script,
        target_placeholder,
    )
}
