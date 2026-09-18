// This protocol is deliberately independent of arbitrary subprocess text.
use serde_json::Value;

const CODES: &[&str] = &[
    "CODEDROBE_RESTART_REQUIRED",
    "CODEDROBE_VERIFY_FAILED",
    "CODEDROBE_DOM_INCOMPATIBLE",
    "CODEDROBE_PORT_OCCUPIED",
    "CODEDROBE_TARGET_TIMEOUT",
    "CODEDROBE_CDP_TIMEOUT",
    "CODEDROBE_CDP_CONNECTION_FAILED",
    "CODEDROBE_CDP_PROTOCOL_ERROR",
    "CODEDROBE_RENDERER_EVALUATION_FAILED",
    "CODEDROBE_THEME_READ_FAILED",
    "CODEDROBE_THEME_INVALID",
    "TARGET_NOT_FOUND",
    "NOT_CONNECTED",
    "CODEDROBE_COMMAND_FAILED",
];
const CHECKS: &[&str] = &[
    "home-canvas-paint",
    "theme-identity", "css-identity", "images-decoded", "horizontal-layout", "runtime-installed", "style-present", "renderer-profile",
    "host-root",
    "known-scene",
    "scene-composer",
    "conversation-timeline",
    "conversation-editor",
    "canvas-paint",
    "composer-paint",
    "editor-paint",
    "editor-text",
    "assistant-paint",
    "assistant-text",
    "user-paint",
    "menu-paint",
    "dialog-paint",
    "send-paint",
    "send-disc",
    "send-hover",
    "send-disc-hover",
    "single-style-node",
    "root",
    "home-header",
    "home-composer",
    "home-composer-panel",
    "conversation-composer",
    "assistant-shell",
    "projects-shell",
    "expert-shell",
    "skills-shell",
    "connector-shell",
    "automation-shell",
    "project-chat-shell",
    "project-chat-composer",
];

pub(crate) struct Diagnostic {
    pub code: &'static str,
    pub checks: Vec<&'static str>,
}
impl Diagnostic {
    pub fn log_code(&self, exit: i32) -> String {
        let mut text = format!("{}-EXIT{}", self.code, exit);
        if !self.checks.is_empty() {
            text.push_str(&format!("-CHECKS-{}", self.checks.join("+")));
        }
        text
    }
}
pub(crate) fn parse(output: &str) -> Option<Diagnostic> {
    output
        .lines()
        .filter_map(|line| {
            let body = line.strip_prefix("[codedrobe-diagnostic] ")?;
            if body.len() > 4096 {
                return None;
            }
            let value: Value = serde_json::from_str(body).ok()?;
            if value.get("version")?.as_u64()? != 1 {
                return None;
            }
            let code = *CODES
                .iter()
                .find(|code| Some(**code) == value.get("code").and_then(Value::as_str))?;
            let mut checks: Vec<_> = value
                .get("checks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    CHECKS
                        .iter()
                        .copied()
                        .find(|name| Some(*name) == item.as_str())
                })
                .collect();
            checks.sort_unstable();
            checks.dedup();
            Some(Diagnostic { code, checks })
        })
        .last()
}

pub(crate) fn deterministic(code: &str) -> bool {
    matches!(
        code,
        "CODEDROBE_DOM_INCOMPATIBLE"
            | "COMPAT_STYLE_MISMATCH"
            | "COMPAT_STRUCTURE_UNSUPPORTED"
            | "COMPAT_PALETTE_UNSUPPORTED"
            | "COMPAT_IMAGE_INVALID"
            | "APPLY_BASELINE_UNKNOWN"
            | "APPLY_RECOVERY_PENDING"
            | "CDP_TARGET_AMBIGUOUS"
            | "CODEDROBE_THEME_READ_FAILED"
            | "CODEDROBE_THEME_INVALID"
            | "CODEDROBE_VERIFY_FAILED"
            | "CODEDROBE_NOT_FOUND"
            | "NODE_NOT_FOUND"
            | "CODEDROBE_RESTART_REQUIRED"
            | "UNSUPPORTED_WORKBUDDY_VERSION"
    )
}

pub(crate) fn message(code: &str) -> &'static str {
    match code {
        "CODEDROBE_RESTART_REQUIRED" => {
            "WorkBuddy 需要重启后才能开启本地 CDP；请保存工作后确认重启。"
        }
        "CODEDROBE_VERIFY_FAILED" => {
            "主题应用后的校验未通过，已停止自动重试。请恢复原版或更换主题。"
        }
        "CODEDROBE_DOM_INCOMPATIBLE" => {
            "当前 WorkBuddy 页面的必要组件与主题不匹配。请回到首页重试；仍失败时请更新主题切换器。"
        }
        "CODEDROBE_PORT_OCCUPIED" => "本地 CDP 端口已被其他程序占用。",
        "CODEDROBE_THEME_READ_FAILED" | "CODEDROBE_THEME_INVALID" => {
            "主题资源无法读取或格式无效。请重新安装完整的主题切换器；自定义主题可从主题库重新保存。"
        }
        "CODEDROBE_TIMEOUT" | "CODEDROBE_CDP_TIMEOUT" | "CODEDROBE_TARGET_TIMEOUT" => {
            "WorkBuddy 本地连接响应超时。请等待窗口加载完成后重试。"
        }
        "CODEDROBE_CDP_CONNECTION_FAILED" | "TARGET_NOT_FOUND" | "NOT_CONNECTED" => {
            "未连接到可用的 WorkBuddy 页面，请检查连接状态后重试。"
        }
        "CODEDROBE_CDP_PROTOCOL_ERROR" | "CODEDROBE_RENDERER_EVALUATION_FAILED" => {
            "WorkBuddy 页面执行主题检查失败。请重试；持续失败时请提供此错误码。"
        }
        _ => "本地主题操作未完成，请在设置与诊断中查看错误码。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_diagnostic_is_bounded_and_allowlisted() {
        let output = "private target title\n[codedrobe-diagnostic] {\"version\":1,\"code\":\"CODEDROBE_DOM_INCOMPATIBLE\",\"checks\":[\"private-name\",\"home-composer-panel\",\"home-composer-panel\"],\"message\":\"private path\"}";
        let result = parse(output).unwrap();
        assert_eq!(result.code, "CODEDROBE_DOM_INCOMPATIBLE");
        assert_eq!(result.checks, vec!["home-composer-panel"]);
        assert!(!result.log_code(1).contains("private"));
    }

    #[test]
    fn invalid_envelopes_and_raw_error_mentions_are_not_trusted() {
        for text in [
            "PRIVATE-TITLE-CODEDROBE_DOM_INCOMPATIBLE",
            "[codedrobe-diagnostic] {\"version\":2,\"code\":\"CODEDROBE_DOM_INCOMPATIBLE\"}",
            "[codedrobe-diagnostic] {\"version\":1,\"code\":\"secret-value\"}",
            "[codedrobe-diagnostic] broken",
        ] {
            assert!(parse(text).is_none());
        }
        assert!(parse(&format!("[codedrobe-diagnostic] {}", "x".repeat(8192))).is_none());
    }
}
