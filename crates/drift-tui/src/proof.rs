use lunatui_core::{ClusterTable, Grid};

pub fn capture(name: &str, grid: &Grid, clusters: &ClusterTable) {
    if std::env::var_os("DRIFT_CAPTURE").is_none() {
        return;
    }
    let theme = crate::diff_palette::active_theme();
    let defaults = format!(
        "{:?} {:?}",
        theme.resolve(lunatui_theme::Token::TextPrimary),
        theme.resolve(lunatui_theme::Token::Surface0)
    );
    let colors: Vec<Vec<String>> = (0..grid.height())
        .map(|y| {
            (0..grid.width())
                .map(|x| {
                    let style = grid.get(x, y).unwrap().style();
                    format!("{:?} {:?}", style.fg, style.bg)
                })
                .collect()
        })
        .collect();
    println!(
        "DRIFT_FRAME {}",
        serde_json::json!({"name":name,"text":grid.display_text(clusters),"colors":colors,"defaults":defaults})
    );
}
