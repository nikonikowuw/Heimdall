//! 列表查询共用的关键字模式构造。
//!
//! 关键字一律按字面量匹配：用户输入里的 `%` / `_` / `\` 必须被转义，
//! 否则 `100%` 会被 SQLite 当成前缀通配符，既返回错误结果又让 `LIKE` 退化为全表扫描。

use sea_orm::sea_query::LikeExpr;

/// 转义 LIKE 通配符，让用户输入按字面量匹配而不是被当成模式
pub(crate) fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 构造 `%关键字%` 的字面量 LIKE 模式；None、空串与纯空白均视为不过滤。
///
/// 转义符与 [`escape_like`] 的转义字符保持一致，两处必须同源——分散在各仓储里手写
/// `escape('\\')` 时，改一处漏一处会静默地把通配符语义漏回给客户端。
pub(crate) fn keyword_pattern(keyword: Option<&str>) -> Option<LikeExpr> {
    let keyword = keyword.map(str::trim).filter(|raw| !raw.is_empty())?;
    Some(LikeExpr::new(format!("%{}%", escape_like(keyword))).escape('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_like_escapes_wildcards_and_backslash() {
        assert_eq!(escape_like("plain"), "plain");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("c:\\tmp"), "c:\\\\tmp");
    }

    #[test]
    fn keyword_pattern_skips_blank_input() {
        assert!(keyword_pattern(None).is_none());
        assert!(keyword_pattern(Some("")).is_none());
        assert!(keyword_pattern(Some("   ")).is_none());
        assert!(keyword_pattern(Some("  Front Gate  ")).is_some());
    }
}
