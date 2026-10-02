//! Explicit public-geocoder probe: cargo run -p smpl-calendar --example city_search_probe -- Lafayete
#[allow(dead_code)]
#[path = "../src/cities.rs"]
mod cities;
#[allow(dead_code)]
#[path = "../src/weather.rs"]
mod weather;

fn main() -> anyhow::Result<()> {
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    for query in std::env::args().skip(1) {
        let results = cities::search(&query, &cancelled)?;
        println!(
            "{query}: suggested={} warning={:?}",
            results.suggested, results.warning
        );
        for city in results.cities {
            println!(
                "  {} [{}, {}; {}]",
                city.label, city.latitude, city.longitude, city.timezone
            );
        }
    }
    Ok(())
}
