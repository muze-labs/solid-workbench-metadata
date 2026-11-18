use std::{io::Cursor, sync::LazyLock, time::Duration};
use tokio::time::{Instant, timeout_at};

use image::{ImageFormat, ImageReader, Limits};
use scraper::{Html, Selector};
use serde::Deserialize;
use url::Url;

use crate::outbound::{self, Body};

static BASE: LazyLock<Selector> = LazyLock::new(|| selector("head base[href]"));
static TITLE: LazyLock<Selector> = LazyLock::new(|| selector("head title"));
static LINKS: LazyLock<Selector> = LazyLock::new(|| selector("head link[rel][href]"));

fn selector(value: &str) -> Selector {
    Selector::parse(value).unwrap()
}

#[derive(Default, Deserialize)]
struct Manifest {
    short_name: Option<String>,
    name: Option<String>,
    #[serde(default)]
    icons: Vec<ManifestIcon>,
}

#[derive(Deserialize)]
struct ManifestIcon {
    src: Option<String>,
    purpose: Option<String>,
}

#[derive(Default)]
struct Page {
    title: Option<String>,
    manifest: Option<Url>,
    icons: Vec<Url>,
}

fn title(value: &str) -> Option<String> {
    let cleaned: String = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(180)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn parse_page(url: &Url, content: &[u8]) -> Page {
    let document = Html::parse_document(&String::from_utf8_lossy(content));
    let base = document
        .select(&BASE)
        .next()
        .and_then(|element| url.join(element.value().attr("href")?).ok());
    let base = base.as_ref().unwrap_or(url);
    let mut page = Page {
        title: document
            .select(&TITLE)
            .next()
            .and_then(|element| title(&element.text().collect::<String>())),
        ..Page::default()
    };
    for link in document.select(&LINKS) {
        let element = link.value();
        let Some(url) = element.attr("href").and_then(|href| base.join(href).ok()) else {
            continue;
        };
        let relations = element.attr("rel").unwrap_or("").split_ascii_whitespace();
        for relation in relations {
            if relation.eq_ignore_ascii_case("manifest") && page.manifest.is_none() {
                page.manifest = Some(url.clone());
            }
            if relation.eq_ignore_ascii_case("icon") && page.icons.len() < 2 {
                page.icons.push(url.clone());
            }
        }
    }
    page
}

pub async fn discover(input: Url) -> (String, Option<Vec<u8>>) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let hostname = input.host_str().unwrap_or("Website").to_owned();
    let (document, page) = match timeout_at(deadline, outbound::get(input, Body::Html)).await {
        Ok(Some((document, bytes))) => {
            let page = parse_page(&document, &bytes);
            (document, page)
        }
        _ => return (hostname, None),
    };
    let mut title = page.title;
    let mut icons = Vec::new();
    if let Some(manifest_url) = page.manifest {
        let manifest_deadline = deadline.min(Instant::now() + Duration::from_secs(2));
        if let Ok(Some((manifest_url, bytes))) =
            timeout_at(manifest_deadline, outbound::get(manifest_url, Body::Json)).await
        {
            if let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) {
                title = title
                    .or_else(|| manifest.short_name.as_deref().and_then(self::title))
                    .or_else(|| manifest.name.as_deref().and_then(self::title));
                icons.extend(
                    manifest
                        .icons
                        .into_iter()
                        .filter(|icon| {
                            icon.purpose.as_deref().is_none_or(|purpose| {
                                purpose.split_ascii_whitespace().any(|token| token == "any")
                            })
                        })
                        .filter_map(|icon| manifest_url.join(&icon.src?).ok())
                        .take(2),
                );
            }
        }
    }
    icons.extend(page.icons);
    icons.extend(document.join("/favicon.ico"));
    for icon in icons {
        if Instant::now() >= deadline {
            break;
        }
        let fetch_deadline = deadline.min(Instant::now() + Duration::from_millis(1500));
        if let Ok(Some((_, bytes))) =
            timeout_at(fetch_deadline, outbound::get(icon, Body::Image)).await
        {
            let decode = tokio::task::spawn_blocking(move || normalize(&bytes));
            if let Ok(Ok(Some(icon))) = timeout_at(deadline, decode).await {
                return (title.unwrap_or(hostname), Some(icon));
            }
        }
    }
    (title.unwrap_or(hostname), None)
}

fn normalize(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(96, 96);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png).ok()?;
    Some(output.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_and_relative_links_only_from_head() {
        let url = Url::parse("https://example.com/deep/page").unwrap();
        let page = parse_page(
            &url,
            br#"<title> An &amp; app </title>
            <base href="/assets/"><link rel="manifest" href="custom.webmanifest">
            <link rel="shortcut ICON" href="mark.ico"><body>
            <link rel="icon" href="body.png">"#,
        );
        assert_eq!(page.title.as_deref(), Some("An & app"));
        assert_eq!(
            page.manifest.unwrap().as_str(),
            "https://example.com/assets/custom.webmanifest"
        );
        assert_eq!(page.icons, [url.join("/assets/mark.ico").unwrap()]);
    }

    #[test]
    fn normalizes_a_raster_and_rejects_non_images() {
        let png = image::DynamicImage::new_rgba8(160, 80);
        let mut bytes = Cursor::new(Vec::new());
        png.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let normalized = normalize(bytes.get_ref()).unwrap();
        let image = image::load_from_memory(&normalized).unwrap();
        assert_eq!((image.width(), image.height()), (96, 48));
        assert!(normalize(b"<svg></svg>").is_none());
    }
}
