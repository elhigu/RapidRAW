/// mimalloc must not madvise(MADV_HUGEPAGE): on fragmented Linux systems, page faults in
/// large image buffers then stall on synchronous memory compaction (see Cargo.toml).
#[test]
fn large_allocations_do_not_request_transparent_huge_pages() {
    let buffers: Vec<Vec<u8>> = (0..4)
        .map(|_| {
            let mut buffer = vec![0u8; 64 << 20];
            for byte in buffer.iter_mut().step_by(4096) {
                *byte = 1;
            }
            buffer
        })
        .collect();

    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("read /proc/self/smaps");
    let marked = smaps
        .lines()
        .filter(|line| line.starts_with("VmFlags:"))
        .filter(|line| line.split_whitespace().any(|flag| flag == "hg"))
        .count();
    std::hint::black_box(&buffers);

    assert_eq!(
        marked, 0,
        "{marked} memory mappings are marked MADV_HUGEPAGE; build mimalloc with the `no_thp` feature"
    );
}
