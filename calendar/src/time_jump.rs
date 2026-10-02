#[derive(Debug, PartialEq)]
pub struct Match {
    pub query: String,
    pub index: i32,
    pub message: String,
}

pub fn is_input(text: &str) -> bool {
    text.len() == 1 && (text.as_bytes()[0].is_ascii_digit() || text == ":")
}

pub fn edit(current: &str, input: &str, backspace: bool) -> Match {
    let mut query: String = current.chars().take(5).collect();
    let overflow = !backspace && is_input(input) && query.len() >= 5;
    if backspace {
        query.pop();
    } else if is_input(input) && query.len() < 5 {
        query.push_str(input);
    }
    let index = if overflow {
        None
    } else {
        matching_slot(&query)
    };
    let message = match index {
        Some(index) => format!("{} → {:02}:{:02}", query, index / 12, index % 12 * 5),
        None if overflow => "Too long; use HH:MM".into(),
        None if query.is_empty() => "Type a time, e.g. 21:30".into(),
        None => format!("{query}: no matching 5-minute time"),
    };
    Match {
        query,
        index: index.map_or(-1, |i| i as i32),
        message,
    }
}

fn matching_slot(query: &str) -> Option<u32> {
    if query.is_empty() || !query.bytes().all(|v| v.is_ascii_digit() || v == b':') {
        return None;
    }
    let (hour, minute) = if let Some((hours, minutes)) = query.split_once(':') {
        if hours.is_empty() || hours.len() > 2 || minutes.len() > 2 {
            return None;
        }
        let hour = hours.parse::<u32>().ok()?;
        let minute = match minutes.len() {
            0 => 0,
            1 => minutes.parse::<u32>().ok()? * 10,
            _ => minutes.parse::<u32>().ok()?,
        };
        (hour, minute)
    } else {
        match query.len() {
            1 | 2 => (query.parse::<u32>().ok()?, 0),
            3 if query[..2].parse::<u32>().ok()? <= 23 => (
                query[..2].parse().ok()?,
                query[2..].parse::<u32>().ok()? * 10,
            ),
            3 => (query[..1].parse().ok()?, query[1..].parse().ok()?),
            4 => (query[..2].parse().ok()?, query[2..].parse().ok()?),
            _ => return None,
        }
    };
    (hour < 24 && minute < 60 && minute % 5 == 0).then_some(hour * 12 + minute / 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_hours_and_minutes_jump_to_real_options() {
        for (query, expected) in [
            ("2", 24),
            ("21", 252),
            ("213", 258),
            ("2130", 258),
            ("21:", 252),
            ("21:3", 258),
            ("21:30", 258),
            ("9", 108),
            ("09", 108),
            ("930", 114),
            ("0930", 114),
            ("9:30", 114),
            ("00:00", 0),
            ("2355", 287),
        ] {
            assert_eq!(matching_slot(query), Some(expected), "{query}");
        }
    }

    #[test]
    fn invalid_and_off_grid_values_do_not_choose_a_nearby_time() {
        for query in [
            "", "24", "24:00", "21:31", "2131", "2360", ":30", "9::3", "12345", "٢١", "-1",
        ] {
            assert_eq!(matching_slot(query), None, "{query}");
        }
        assert_eq!(edit("21:3", "1", false).index, -1);
    }

    #[test]
    fn editing_is_bounded_and_backspace_can_recover() {
        let typed = edit("21:3", "0", false);
        assert_eq!((typed.query.as_str(), typed.index), ("21:30", 258));
        assert_eq!(edit("21:30", "9", false).query.len(), 5);
        assert_eq!(edit("21:30", "9", false).index, -1);
        assert_eq!(edit("21:31", "", true).index, 258);
        assert_eq!(edit("2", "", true).query, "");
        for text in ["x", "-", "12", "", "\u{001b}"] {
            assert!(!is_input(text));
        }
    }
}
