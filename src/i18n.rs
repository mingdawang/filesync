/// 国际化支持：UI 语言跟随系统语言设置。
///
/// 启动时检测一次系统语言，之后通过 `t(zh, en)` 选择对应字符串。
/// Windows 系统语言为中文（zh-*）时显示中文，其余均显示英文。

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// 返回当前语言（启动时检测一次，之后缓存）。
pub fn lang() -> Lang {
    *LANG.get_or_init(detect_lang)
}

/// 当前语言是否为中文。
#[inline]
pub fn is_zh() -> bool {
    lang() == Lang::Zh
}

/// 根据系统语言返回对应字符串。
#[inline]
pub fn t(zh: &'static str, en: &'static str) -> &'static str {
    if is_zh() { zh } else { en }
}

// ─────────────────────────────────────────────────────────────────
// 语言检测
// ─────────────────────────────────────────────────────────────────

fn detect_lang() -> Lang {
    #[cfg(windows)]
    {
        // GetUserDefaultUILanguage 返回 LANGID（u16）。
        // Primary language ID = LANGID & 0x3FF。
        // LANG_CHINESE = 0x04，涵盖 zh-CN (0x0804)、zh-TW (0x0404) 等所有中文变体。
        #[link(name = "kernel32")]
        extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        if lang_id & 0x3FF == 0x04 {
            return Lang::Zh;
        }
    }

    // 非 Windows：读取 LANG 环境变量作为后备
    #[cfg(not(windows))]
    {
        if std::env::var("LANG")
            .unwrap_or_default()
            .starts_with("zh")
        {
            return Lang::Zh;
        }
    }

    Lang::En
}
