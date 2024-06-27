use assert_cmd::Command;

#[test]
fn dummy_test() {
    let mut cmd = Command::cargo_bin("pharia-skill publish").unwrap();
    cmd.assert().success();
}

