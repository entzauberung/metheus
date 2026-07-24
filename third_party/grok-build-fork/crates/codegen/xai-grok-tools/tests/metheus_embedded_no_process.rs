#[test]
fn embedded_grep_cannot_spawn_rg_or_any_other_process() {
    let source = include_str!("../src/implementations/metheus_embedded.rs");
    for forbidden in ["std::process", "tokio::process", "Command::new("] {
        assert!(
            !source.contains(forbidden),
            "embedded filesystem policy introduced process API: {forbidden}"
        );
    }
}
