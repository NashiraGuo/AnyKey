/// AnyKey engine - Utility functions

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Mutex;

/// Truly lazy regex cache: patterns are compiled once and reused.
static RE_CACHE: Lazy<Mutex<HashMap<&'static str, Regex>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Compile/Cache a regex by literal pattern (must be &'static str for caching safety).
pub fn regex_lazy(pattern: &'static str) -> Regex {
    let mut cache = RE_CACHE.lock().unwrap();
    if let Some(re) = cache.get(pattern) {
        return re.clone();
    }
    let re = Regex::new(pattern).expect("invalid regex pattern");
    cache.insert(pattern, re.clone());
    re
}

/// Normalize key name
pub fn norm_key(key: &str) -> String {
    let k = key.trim().to_lowercase();
    let map = get_norm_map();
    map.get(&k).cloned().unwrap_or(k)
}

fn get_norm_map() -> &'static HashMap<String, String> {
    static MAP: Lazy<HashMap<String, String>> = Lazy::new(|| {
        let mut m = HashMap::new();
        m.insert("esc".into(), "escape".into());
        m.insert("del".into(), "delete".into());
        m.insert("ins".into(), "insert".into());
        m.insert("pgup".into(), "pageup".into());
        m.insert("pgdn".into(), "pagedown".into());
        m.insert("prtsc".into(), "printscreen".into());
        m.insert("scrlk".into(), "scrolllock".into());
        m.insert("capslk".into(), "capslock".into());
        m.insert("appskey".into(), "apps".into());
        m.insert("lcontrol".into(), "lctrl".into());
        m.insert("rcontrol".into(), "rctrl".into());
        m.insert("pagedown".into(), "pgdn".into());
        m.insert("pageup".into(), "pgup".into());
        m.insert("return".into(), "enter".into());
        m
    });
    &MAP
}

pub fn is_single_key(output: &str) -> bool {
    let re = regex_lazy(r"^\{[^}]+\}$");
    re.is_match(output)
}

/// 键名缩写 → 规范全名（小写）映射。
/// 来源：ahk_generator.py / ahi_generator.py 的 COMMON_KEYS_ALIASES，
/// 并补充用户常用缩写（如 bs→backspace）。
/// 用于把 {pgdn}/{bs} 等缩写规范化为 {pagedown}/{backspace}，使落盘 config 保持一致。
fn key_alias_to_full() -> &'static HashMap<&'static str, &'static str> {
    static M: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
        let mut m = HashMap::new();
        m.insert("esc", "escape");
        m.insert("del", "delete");
        m.insert("ins", "insert");
        m.insert("pgup", "pageup");
        m.insert("pgdn", "pagedown");
        m.insert("prtsc", "printscreen");
        m.insert("scrlk", "scrolllock");
        m.insert("capslk", "capslock");
        m.insert("appskey", "apps");
        m.insert("bs", "backspace");   // 用户常用缩写
        m
    });
    &M
}

/// 键盘上「不需要 Shift」的基础符号。
/// 按用户规则（2026-07-08），裸键名允许的基础符号仅限这些；
/// Shift 符号（!@#$%^&*()_+{}|:"<>?~）与大写字母均不算键名裸字符。
fn is_basic_symbol(c: char) -> bool {
    matches!(c, '`' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | ',' | '.' | '/')
}

/// 判断裸输出值是否为「单个键名」（应包裹为 {X}）。
/// 新规则（用户 2026-07-08）：
///  - 只能是【一个字符】；
///  - 字符属于：小写字母 a-z / 数字 0-9 / 键盘基础符号（无 Shift）；
///  - 大写字母、Shift 符号、多字符（除非已用 {} 包裹）一律视为文本。
pub fn is_bare_key_name(val: &str) -> bool {
    if val.is_empty() { return false; }
    if is_single_key(val) { return true; }            // 已是 {X} 形式 → 是键
    if val.chars().count() != 1 { return false; }     // 多字符裸值 = 文本（需用户自包 {}）
    let c = val.chars().next().unwrap();
    c.is_ascii_lowercase() || c.is_ascii_digit() || is_basic_symbol(c)
}

/// 规范键名（去花括号后的内部名）：缩写→全名，未知名转小写以保持 config 一致。
pub fn canonical_key_name(inner: &str) -> String {
    let lower = inner.to_ascii_lowercase();
    if let Some(full) = key_alias_to_full().get(lower.as_str()) {
        return (*full).to_string();
    }
    lower
}

/// 归一化输出值：
///  - 空 → 空
///  - 已 {X} → 规范化内部键名（缩写→全名、全名转小写），如 {pgdn}→{pagedown}、{bs}→{backspace}、{Backspace}→{backspace}
///  - 裸单字符键名（小写/数字/基础符号）→ 包成 {X}
///  - 其余（多字符裸值 / 文本 / 宏）→ 原样
pub fn is_func_call(output: &str) -> bool {
    let re = regex_lazy(r"^[A-Za-z_]\w*\(.+\)$");
    re.is_match(output)
}

pub fn normalize_layer_name(val: &str) -> String {
    match val.trim() {
        "0" => "base",
        "1" => "fn1",
        "2" => "fn2",
        "3" => "fn3",
        "4" => "fn4",
        "5" => "fn5",
        "6" => "fn6",
        "7" => "fn7",
        "8" => "fn8",
        "9" => "fn9",
        s => s,
    }.to_lowercase()
}

pub fn extract_layer_name(output: &str) -> String {
    let re = regex_lazy(r"^\{?([fbt]n\d+)\}?$");
    if let Some(caps) = re.captures(output) {
        let raw = caps[1].to_lowercase();
        if raw.starts_with("bn") || raw.starts_with("tn") {
            return format!("fn{}", &raw[2..]);
        }
        return raw;
    }
    output.trim_matches(|c| c == '{' || c == '}').to_string()
}

/// Check if output is a layer key — accepts both `{fn1}` and `fn1` forms.
/// Recognizes `{fnX}` / `{bnX}` / `{tnX}` (toggle layer).
/// Used by pipeline.rs (braced) and main.rs drain_emit_log (bare).
pub fn is_layer_key(output: &str) -> bool {
    let re = regex_lazy(r"^\{?[fbt]n\d+\}?$");
    re.is_match(output)
}

pub fn parse_u64_opt(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() { None } else { s.parse::<u64>().ok() }
}

pub fn strip_slot_suffix(key: &str) -> String {
    let re = regex_lazy(r"_\d+$");
    re.replace(key, "").to_string()
}
