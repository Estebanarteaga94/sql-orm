#[test]
fn active_record_ui() {
    let tests = trybuild::TestCases::new();

    tests.pass("tests/ui/active_record_public_valid.rs");
    tests.pass("tests/ui/active_record_delete_public_valid.rs");
    tests.pass("tests/ui/active_record_save_public_valid.rs");
    tests.compile_fail("tests/ui/active_record_missing_entity_set.rs");
}
