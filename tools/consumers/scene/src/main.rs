fn main() -> baozi::Result<()> {
    let expected = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "obj,stl".to_owned());
    baozi_scene_consumer::verify(&baozi_scene_consumer::expected_formats(&expected))?;
    println!("Scene consumer passed: {expected}");
    Ok(())
}
