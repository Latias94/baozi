#[test]
fn public_scene_contract() {
    let expected = std::env::var("BAOZI_EXPECT_FORMATS").unwrap_or_else(|_| "obj,stl".to_owned());
    baozi_scene_consumer::verify(&baozi_scene_consumer::expected_formats(&expected)).unwrap();
}
