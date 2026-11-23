fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let page: usize = a[2].parse().unwrap();
    let mut c = ibis_pdf::PdfContainer::from_bytes(data).unwrap();
    let mut i = 0;
    ibis_pdf::interpret::visit_text_scenes(&mut c, |s| {
        if i == page {
            let mut idx = s.text_index.clone();
            if let Some(lo) = a.get(3) { let lo: f32 = lo.parse().unwrap(); let hi: f32 = a[4].parse().unwrap(); idx.chars.retain(|c| c.rect_pt.x0 >= lo && c.rect_pt.x0 <= hi); }
            print!("{}", ibis_pdf::text_layout::page_text(&idx, ibis_pdf::text_layout::TextLayout::Reading));
        }
        i += 1;
        Ok(())
    }).unwrap();
}
