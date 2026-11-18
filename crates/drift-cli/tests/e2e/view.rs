use crate::harness::Screen;

pub fn left(row: &str) -> &str {
    match row.find(" │ ").or_else(|| row.find(" | ")) {
        Some(at) => &row[..at],
        None => row,
    }
}

pub fn selected(screen: &Screen) -> String {
    screen
        .rows()
        .iter()
        .find(|row| row.starts_with('>'))
        .map(|row| left(row).trim().trim_start_matches('>').trim().to_string())
        .unwrap_or_default()
}

pub fn rows_of_tree(screen: &Screen) -> Vec<String> {
    screen
        .rows()
        .iter()
        .filter(|row| {
            left(row).contains("[ ]") || left(row).contains("[x]") || left(row).contains("[-]")
        })
        .map(|row| left(row).trim().trim_start_matches('>').trim().to_string())
        .collect()
}

pub fn mark_of(screen: &Screen, needle: &str) -> Option<char> {
    rows_of_tree(screen)
        .iter()
        .find(|row| row.contains(needle))
        .and_then(|row| {
            let at = row.find('[')?;
            row[at + 1..].chars().next()
        })
}

pub fn has_row(screen: &Screen, needle: &str) -> bool {
    rows_of_tree(screen).iter().any(|row| row.contains(needle))
}

pub fn selected_is(screen: &Screen, needle: &str) -> bool {
    selected(screen).contains(needle)
}

pub fn current_hunk(screen: &Screen) -> Option<usize> {
    screen.rows().iter().find_map(|row| {
        let rest = row.strip_prefix("> ")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

pub fn column_of(screen: &Screen, row_needle: &str, needle: &str) -> Option<u16> {
    let row = screen.rows().iter().find(|row| row.contains(row_needle))?;
    let byte = row.find(needle)?;
    Some(row[..byte].chars().count() as u16)
}

pub fn palette_count(screen: &Screen) -> Option<usize> {
    screen.contains("Command palette").then_some(())?;
    screen.rows().iter().find_map(|row| {
        let at = row.find(" commands")?;
        row[..at].split_whitespace().last()?.parse().ok()
    })
}

pub fn palette_first(screen: &Screen) -> Option<String> {
    let rows = screen.rows();
    let at = rows.iter().position(|row| row.contains("│›"))?;
    let row = rows.get(at + 1)?;
    let start = row.find('│')? + '│'.len_utf8();
    let end = row.rfind('│')?;
    Some(row[start..end].trim().to_string())
}
