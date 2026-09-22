//! 列表端点共用的查询参数解析。
//!
//! `q` 关键字在多个列表端点（操作审计、运维事件、告警、抓拍、识别）上语义一致：
//! 空白视为未过滤，超长直接拒绝。解析规则只在这里定义一次——各端点各写一份时，
//! 长度上限会漏掉一两处（`q` 走 `LIKE '%...%'` 无法命中索引，上限正是它的代价约束）。

use crate::error::ApiError;

/// 关键字长度上限：LIKE 模式不能由客户端无限拉长
pub const MAX_KEYWORD_CHARS: usize = 64;

/// 解析 `q` 查询参数，空串视为未过滤，超长直接拒绝
pub fn parse_keyword(value: Option<&str>) -> Result<Option<&str>, ApiError> {
    let Some(keyword) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    if keyword.chars().count() > MAX_KEYWORD_CHARS {
        return Err(ApiError::BadRequest(format!(
            "q 长度不能超过 {MAX_KEYWORD_CHARS} 个字符"
        )));
    }
    Ok(Some(keyword))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_keyword_trims_and_bounds_the_pattern() {
        assert!(parse_keyword(None).unwrap().is_none());
        assert!(parse_keyword(Some("   ")).unwrap().is_none());
        assert_eq!(parse_keyword(Some("  cameras  ")).unwrap(), Some("cameras"));

        let at_limit = "a".repeat(MAX_KEYWORD_CHARS);
        assert_eq!(
            parse_keyword(Some(&at_limit)).unwrap(),
            Some(at_limit.as_str())
        );

        let over_limit = "a".repeat(MAX_KEYWORD_CHARS + 1);
        let rejected = parse_keyword(Some(&over_limit)).unwrap_err();
        assert!(rejected.to_string().contains("q"));
    }
}
