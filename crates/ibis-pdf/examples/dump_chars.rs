fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let page: usize = a[2].parse().unwrap();
    let mut c = ibis_pdf::PdfContainer::from_bytes(data).unwrap();
    let mut i = 0;
    ibis_pdf::interpret::visit_text_scenes(&mut c, |s| {
        if i == page {
            for ch in &s.text_index.chars {
                let r = ch.rect_pt;
                println!("{:?} x0={:.1} x1={:.1} top={:.1} base={:.1} op={} v={}", ch.ch, r.x0, r.x1, r.y0, r.y1, ch.stream_op_index, ch.vertical);
            }
        }
        i += 1;
        Ok(())
    }).unwrap();
}
