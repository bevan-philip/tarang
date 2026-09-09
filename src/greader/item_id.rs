use super::GReaderError;

pub fn parse_item_id(raw: &str) -> Result<i64, GReaderError> {
    let err = || GReaderError::BadRequest(format!("invalid item id: {raw}"));

    if let Some(hex) = raw.strip_prefix("tag:google.com,2005:reader/item/") {
        return u64::from_str_radix(hex, 16)
            .map(|v| v as i64)
            .map_err(|_| err());
    }

    if let Ok(v) = raw.parse::<i64>() {
        return Ok(v);
    }

    if raw.len() == 16 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return u64::from_str_radix(raw, 16)
            .map(|v| v as i64)
            .map_err(|_| err());
    }

    Err(err())
}

pub fn format_item_id_long(pk: i64) -> String {
    format!("tag:google.com,2005:reader/item/{:016x}", pk as u64)
}

pub fn format_item_id_decimal(pk: i64) -> String {
    pk.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_long_form() {
        let long = format_item_id_long(1234);
        assert_eq!(parse_item_id(&long).unwrap(), 1234);
    }

    #[test]
    fn accepts_bare_hex() {
        assert_eq!(parse_item_id("00000000000004d2").unwrap(), 1234);
    }

    #[test]
    fn accepts_decimal() {
        assert_eq!(parse_item_id("1234").unwrap(), 1234);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_item_id("not-an-id").is_err());
    }

    #[test]
    fn sixteen_digit_decimal_is_not_misparsed_as_hex() {
        assert_eq!(parse_item_id("1234567890123456").unwrap(), 1234567890123456);
    }
}
