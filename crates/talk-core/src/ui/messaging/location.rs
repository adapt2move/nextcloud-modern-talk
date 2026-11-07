//! Strict, bounded geo URI handling for deliberate map handoff. No network I/O.

/// Return a validated latitude/longitude pair. Only the unambiguous base geo URI
/// form is accepted; unknown parameters and out-of-range coordinates stay generic.
pub fn coordinates(id: &str) -> Option<(&str, &str)> {
    fn valid(value: &str, limit: f64) -> bool {
        !value.is_empty()
            && value.len() <= 32
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'.'))
            && value
                .parse::<f64>()
                .is_ok_and(|n| n.is_finite() && n.abs() <= limit)
    }
    let (lat, lon) = id.strip_prefix("geo:")?.split_once(',')?;
    (valid(lat, 90.) && valid(lon, 180.)).then_some((lat, lon))
}

pub fn map_url(id: &str) -> Option<String> {
    let (lat, lon) = coordinates(id)?;
    Some(format!(
        "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_bounded_coordinates_without_url_injection() {
        assert_eq!(
            coordinates("geo:52.5450511,13.3741463"),
            Some(("52.5450511", "13.3741463"))
        );
        for id in [
            "geo:91,0",
            "geo:0,-181",
            "geo:NaN,0",
            "geo:1,2&evil=1",
            "geo:1,2;u=3",
            "geo:1,2,3",
            "https://example.test/",
        ] {
            assert!(map_url(id).is_none(), "{id}");
        }
        assert_eq!(
            map_url("geo:-12.5,45").unwrap(),
            "https://www.openstreetmap.org/?mlat=-12.5&mlon=45#map=16/-12.5/45"
        );
    }
}
