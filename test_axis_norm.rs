use glam::Vec3;

fn convert_position_zup_to_yup(pos: Vec3) -> Vec3 {
    Vec3::new(
        pos.y,
        pos.z,
        -pos.x,
    )
}

fn main() {
    // Test axis magnitude preservation
    let normalized_axis = Vec3::new(0.577350, 0.577350, 0.577350); // (1,1,1).normalize()
    println!("Original axis: {:?}, length: {}", normalized_axis, normalized_axis.length());
    
    let converted = convert_position_zup_to_yup(normalized_axis);
    println!("Converted axis: {:?}, length: {}", converted, converted.length());
    
    // Test with small axis
    let small_axis = Vec3::new(0.001, 0.002, -0.003).normalize();
    println!("\nSmall axis: {:?}, length: {}", small_axis, small_axis.length());
    
    let converted_small = convert_position_zup_to_yup(small_axis);
    println!("Converted small: {:?}, length: {}", converted_small, converted_small.length());
}
