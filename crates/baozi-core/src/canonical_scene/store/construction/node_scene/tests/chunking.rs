use crate::provenance::{MAX_RESOURCE_IDENTITY_LEN, ResourceIdentity};

use super::super::{process_slice_in_chunks, process_str_in_chunks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stop;

#[test]
fn transform_slice_observes_each_chunk_before_copying_it() {
    let values = vec![1.0_f32; 513];
    let mut observations = 0;
    let mut copied = Vec::new();

    let result = process_slice_in_chunks(
        &values,
        256,
        || Stop,
        |_| {
            observations += 1;
            if observations == 2 { Err(Stop) } else { Ok(()) }
        },
        |chunk| {
            copied.extend_from_slice(chunk);
            Ok(())
        },
    );

    assert_eq!(result, Err(Stop));
    assert_eq!(observations, 2);
    assert_eq!(copied, values[..256]);
}

#[test]
fn lineage_utf8_chunks_preserve_boundaries_and_interleave_work() {
    let identity = ResourceIdentity::new(format!("{}ab", "界".repeat(170)))
        .expect("maximum-length Unicode resource identity");
    assert_eq!(identity.as_str().len(), MAX_RESOURCE_IDENTITY_LEN);

    let mut observed = Vec::new();
    let mut written = String::new();
    process_str_in_chunks(
        identity.as_str(),
        256,
        || Stop,
        |count| {
            observed.push(count);
            Ok(())
        },
        |chunk| {
            written.push_str(chunk);
            Ok(())
        },
    )
    .expect("valid UTF-8 must have a bounded chunk partition");
    assert_eq!(observed, [255, 256, 1]);
    assert_eq!(written, identity.as_str());

    let mut observations = 0;
    let mut prefix = String::new();
    let result = process_str_in_chunks(
        identity.as_str(),
        256,
        || Stop,
        |_| {
            observations += 1;
            if observations == 2 { Err(Stop) } else { Ok(()) }
        },
        |chunk| {
            prefix.push_str(chunk);
            Ok(())
        },
    );
    assert_eq!(result, Err(Stop));
    assert_eq!(observations, 2);
    assert_eq!(prefix, "界".repeat(85));
}
