//! Histogram of retail collision surface ids in a .skate map (audio material RE).
fn main() {
    for path in std::env::args().skip(1) {
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&path)).expect("map");
        let mut counts = std::collections::BTreeMap::<u16, usize>::new();
        for e in map.extensions.iter().filter(|e| e.tag == *b"RWCM") {
            skate_data::retail_collision::visit_clusters(&e.payload, |_, cluster| {
                for t in cluster {
                    *counts.entry(t.surface).or_default() += 1;
                }
                Ok(())
            })
            .expect("RWCM");
        }
        println!("{} {} surface ids", map.name, counts.len());
        for (tag, n) in counts.iter() {
            println!("{tag:#06x} {:>3} {:>3} {n}", (tag >> 7) & 31, tag & 127);
        }
    }
}
