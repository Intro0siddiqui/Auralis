use super::html::*;

#[test]
fn test_html_escape() {
    assert_eq!(
        html_escape("Rock & Roll <script>'\""),
        "Rock &amp; Roll &lt;script&gt;&#39;&quot;"
    );
}

#[test]
fn test_format_time() {
    assert_eq!(format_time(0), "0:00");
    assert_eq!(format_time(65), "1:05");
    assert_eq!(format_time(3600), "60:00");
}

#[test]
fn test_html_escape_hardening() {
    // The five context-critical characters plus backtick/slash hardening.
    // `&#x2F;` decodes to `/` in browsers, so escaped paths render identically.
    assert_eq!(html_escape("&<>\"'"), "&amp;&lt;&gt;&quot;&#39;");
    assert_eq!(html_escape("`"), "&#96;");
    assert_eq!(
        html_escape("/path/to/art.jpg"),
        "&#x2F;path&#x2F;to&#x2F;art.jpg"
    );
    assert_eq!(html_escape("plain"), "plain");
}

#[test]
fn test_render_art_tag() {
    let empty_tag = render_art_tag(None, "Album", "disc-3");
    assert_eq!(empty_tag, r#"<i data-lucide="disc-3"></i>"#);

    let img_tag = render_art_tag(Some("/path/to/art.jpg"), "My Album", "disc-3");
    assert!(img_tag.contains(r#"src="&#x2F;path&#x2F;to&#x2F;art.jpg""#));
    assert!(img_tag.contains(r#"alt="My Album""#));
    assert!(img_tag.contains("embedArt"));
}

#[test]
fn test_render_track_row_html() {
    let html = render_track_row_html(
        "track-1",
        "Midnight City",
        Some("M83"),
        Some("Hurry Up"),
        244,
        Some("https://example.com/art.jpg"),
        true,
    );
    assert!(html.contains(r#"data-track-id="track-1""#));
    assert!(html.contains(r#"data-role="play-row""#));
    assert!(html.contains("Midnight City"));
    assert!(html.contains("M83"));
    assert!(html.contains("4:04"));
    assert!(html.contains("liked"));
    assert!(html.contains("glass-weak"));
}

#[test]
fn test_escape_special_characters_in_row() {
    let html = render_track_row_html(
        "track-2",
        "<script>alert('xss')</script>",
        Some("AC/DC & Friends \"Rock'n'Roll\""),
        Some("Back in Black > White"),
        185,
        None,
        false,
    );
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;&#x2F;script&gt;"));
    assert!(html.contains("AC&#x2F;DC &amp; Friends &quot;Rock&#39;n&#39;Roll&quot;"));
    assert!(html.contains("Back in Black &gt; White"));
    assert!(html.contains("3:05"));
    assert!(!html.contains("liked"));
}

#[test]
fn test_render_search_results_html() {
    let items = vec![
        SearchTrackItem {
            id: "s-1".to_string(),
            title: "Search Song".to_string(),
            artist: Some("Search Artist".to_string()),
            album: Some("Search Album".to_string()),
            duration_secs: 200,
            album_art_path: None,
            is_favorite: true,
        },
        SearchTrackItem {
            id: "s-2".to_string(),
            title: "Another Song".to_string(),
            artist: None,
            album: None,
            duration_secs: 150,
            album_art_path: None,
            is_favorite: false,
        },
    ];

    let html = render_search_results_html(&items);
    assert!(html.contains(r#"data-track-id="s-1""#));
    assert!(html.contains("Search Song"));
    assert!(html.contains("Search Artist"));
    assert!(html.contains(r#"data-track-id="s-2""#));
    assert!(html.contains("Another Song"));
    assert!(html.contains("Unknown Artist"));
}
