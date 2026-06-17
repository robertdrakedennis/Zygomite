    use super::{decode_chunk_instance_stream, decode_map_square};
    use std::collections::BTreeMap;
    use std::iter::repeat_n;

    #[test]
    fn decodes_empty_mapsquare() {
        let files = BTreeMap::new();
        let decoded = decode_map_square(&files, 947).expect("empty mapsquare");
        assert!(decoded.environment.is_none());
        assert!(decoded.lights.is_empty());
        assert!(decoded.water.is_empty());
    }

    #[test]
    fn decodes_raw_loc_variant_shapes() {
        let mut files = BTreeMap::new();
        files.insert(0, vec![1, 1, (2 << 2) | 1, 0, 0]);

        let decoded = decode_map_square(&files, 947).expect("loc variants");

        assert_eq!(2, decoded.locs.len());
        assert_eq!(2, decoded.locs[0].shape);
        assert_eq!(1, decoded.locs[0].angle);
        assert!(!decoded.locs[0].derived);
        assert_eq!(0x17, decoded.locs[1].shape);
        assert_eq!(2, decoded.locs[1].angle);
        assert_eq!(2, decoded.locs[1].source_shape);
        assert_eq!(1, decoded.locs[1].source_angle);
        assert!(decoded.locs[1].derived);
    }

    #[test]
    fn decodes_map_file5_metadata_header() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 3];
        payload.extend(repeat_n(0, 66 * 66));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 metadata");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let metadata = decoded
            .map_file5_terrain_metadata
            .expect("map file 5 terrain metadata");

        assert_eq!("current947TerrainMetadata", terrain.format);
        assert_eq!("current947TerrainMetadata", metadata.format);
        assert_eq!(
            "direct maps archive file 5 terrain/config-id metadata",
            terrain.semantic_status.role
        );
        assert_eq!(
            "not proven by current 947 evidence",
            terrain.semantic_status.runtime_application
        );
        assert_eq!(1, terrain.summary.level_count);
        assert_eq!(0, terrain.summary.tile_record_count);
        assert_eq!(0, terrain.summary.extended_tile_record_count);
        assert_eq!(0, terrain.summary.slot1_unique_id_count);
        assert_eq!(0, terrain.summary.slot4_unique_id_count);
        assert_eq!(0, terrain.summary.slot1_tile_reference_count);
        assert_eq!(0, terrain.summary.slot4_tile_reference_count);
        assert!(!terrain.evidence_notes.is_empty());
        assert!(
            terrain
                .warnings
                .iter()
                .any(|warning| warning.contains("decoded metadata only"))
        );
        assert_eq!(vec![b'j', b'a', b'g', b'x', 1], terrain.header);
        assert_eq!(1, terrain.levels.len());
        assert_eq!(3, terrain.levels[0].level);
        assert!(terrain.levels[0].tiles.is_empty());
        assert_eq!(66 * 66 + 1, terrain.payload_bytes);
    }

    #[test]
    fn decodes_map_file5_multiple_levels_until_eof() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        payload.extend(repeat_n(0, 66 * 66));
        payload.push(2);
        payload.extend(repeat_n(0, 66 * 66));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 multiple levels");
        let terrain = decoded.terrain_file5.expect("terrain file 5");

        assert_eq!(2, terrain.levels.len());
        assert_eq!(0, terrain.levels[0].level);
        assert_eq!(2, terrain.levels[1].level);
        assert!(!terrain.truncated);
        assert_eq!(0, terrain.trailing_bytes);
        assert_eq!((66 * 66 + 1) * 2, terrain.payload_bytes);
    }

    #[test]
    fn decodes_map_file5_partial_zero_suffix_until_eof() {
        let mut files = BTreeMap::new();
        let payload = vec![b'j', b'a', b'g', b'x', 1, 0, 1, 0xaa, 0xbb, 0, 0];
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 partial zero suffix");
        let terrain = decoded.terrain_file5.expect("terrain file 5");

        assert_eq!(1, terrain.levels.len());
        assert_eq!(1, terrain.levels[0].tiles.len());
        assert!(!terrain.truncated);
        assert_eq!(0, terrain.trailing_bytes);
    }

    #[test]
    fn decodes_map_file5_current_slot_ids() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 0xaa);
        push_u8(&mut payload, 0xbb);
        push_u8(&mut payload, 11);
        push_u8(&mut payload, 0xcc);
        push_u8(&mut payload, 0xdd);
        push_u8(&mut payload, 21);
        push_u8(&mut payload, 0xee);
        payload.extend(repeat_n(0, 66 * 66 - 1));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 ids");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let level = &terrain.levels[0];

        assert_eq!(vec![10], level.slot1_ids);
        assert_eq!(vec![20], level.slot4_ids);
        assert_eq!(1, terrain.summary.tile_record_count);
        assert_eq!(1, terrain.summary.slot1_unique_id_count);
        assert_eq!(1, terrain.summary.slot4_unique_id_count);
        assert_eq!(1, terrain.summary.slot1_tile_reference_count);
        assert_eq!(1, terrain.summary.slot4_tile_reference_count);
        assert_eq!(1, level.tiles.len());
        assert_eq!(0, level.tiles[0].x);
        assert_eq!(0, level.tiles[0].z);
        assert_eq!(vec![10], level.tiles[0].slot1_ids);
        assert_eq!(vec![20], level.tiles[0].slot4_ids);
    }

    #[test]
    fn decodes_map_file5_extended_slot_ids() {
        let mut files = BTreeMap::new();
        let mut payload = vec![b'j', b'a', b'g', b'x', 1, 0];
        push_u8(&mut payload, 0x10);
        push_u8(&mut payload, 0xaa);
        push_u8(&mut payload, 0xbb);
        push_u8(&mut payload, 0xcc);
        push_u8(&mut payload, 0xdd);
        push_u8(&mut payload, 11);
        push_u8(&mut payload, 0xee);
        push_u8(&mut payload, 0xff);
        push_u8(&mut payload, 21);
        push_u8(&mut payload, 31);
        push_u8(&mut payload, 0x99);
        push_u8(&mut payload, 41);
        payload.extend(repeat_n(0, 66 * 66 - 1));
        files.insert(5, payload);

        let decoded = decode_map_square(&files, 947).expect("file 5 extended ids");
        let terrain = decoded.terrain_file5.expect("terrain file 5");
        let level = &terrain.levels[0];

        assert_eq!(vec![10, 40], level.slot1_ids);
        assert_eq!(vec![20, 30], level.slot4_ids);
        assert_eq!(1, terrain.summary.extended_tile_record_count);
        assert_eq!(2, terrain.summary.slot1_unique_id_count);
        assert_eq!(2, terrain.summary.slot4_unique_id_count);
        assert_eq!(2, terrain.summary.slot1_tile_reference_count);
        assert_eq!(2, terrain.summary.slot4_tile_reference_count);
        assert_eq!(vec![10, 40], level.tiles[0].slot1_ids);
        assert_eq!(vec![30, 20], level.tiles[0].slot4_ids);
    }

    #[test]
    fn decodes_current_environment_sky_reflection_and_colour_grading() {
        let mut files = BTreeMap::new();
        let mut payload = Vec::new();

        push_i32_be(&mut payload, -1);
        push_zero_u16s(&mut payload, 6);
        push_zero_f32s(&mut payload, 3);
        push_i32_be(&mut payload, 0);
        push_u16_be(&mut payload, 0);
        push_u8(&mut payload, 1);
        push_zero_f32s(&mut payload, 4);
        push_u8(&mut payload, 1);
        push_zero_f32s(&mut payload, 4);
        push_zero_f32s(&mut payload, 9);
        push_zero_f32s(&mut payload, 8);
        push_i32_be(&mut payload, 0);
        push_i32_be(&mut payload, 0);
        push_f32_be(&mut payload, 0.0);
        push_u8(&mut payload, 1);
        push_u8(&mut payload, 3);
        push_zero_f32s(&mut payload, 5);
        push_zero_f32s(&mut payload, 6);
        push_i16_be(&mut payload, -1);
        push_i16_be(&mut payload, 77);
        push_i16_be(&mut payload, 11);
        push_f32_be(&mut payload, 0.25);
        push_i16_be(&mut payload, 22);
        push_f32_be(&mut payload, 0.75);
        push_f32_be(&mut payload, 0.5);
        push_f32_be(&mut payload, 1.0);
        push_f32_be(&mut payload, 2.0);
        push_f32_be(&mut payload, 3.0);
        files.insert(6, payload);

        let decoded = decode_map_square(&files, 947).expect("environment");
        let env = decoded.environment.expect("environment data");

        assert_eq!(-1, env.skybox.skybox_id);
        assert_eq!(77, env.skybox.reflection_material_id);
        assert!(env.skybox.reflection_enabled);
        assert_eq!(3, env.colour_remap.entries.len());
        assert_eq!(22, env.colour_remap.entries[0].texture);
        assert_f32_eq(0.75, env.colour_remap.entries[0].weighting);
        assert_eq!(-1, env.colour_remap.entries[1].texture);
        assert_eq!(2, env.colour_remap.packet_pairs.len());
        assert_eq!(11, env.colour_remap.packet_pairs[0].texture);
        assert_eq!(22, env.colour_remap.packet_pairs[1].texture);
        assert_f32_eq(0.5, env.light_probe.unknown1);
        assert_f32_eq(3.0, env.light_probe.unknown2.z);
    }

    #[test]
    fn decodes_water_patch() {
        let mut files = BTreeMap::new();
        let mut payload = Vec::new();
        push_u8(&mut payload, 1);
        push_u16_be(&mut payload, 5);
        push_u16_be(&mut payload, 9);
        push_u8(&mut payload, 1);
        push_i16_be(&mut payload, 128);
        push_u8(&mut payload, 0xff);
        push_u8(&mut payload, 3);
        push_u8(&mut payload, 0xfc);
        push_f32_be(&mut payload, 1.0);
        push_f32_be(&mut payload, 2.0);
        push_f32_be(&mut payload, 3.0);
        push_f32_be(&mut payload, 4.0);
        push_u8(&mut payload, 7);
        push_u8(&mut payload, 0xf8);
        files.insert(8, payload);

        let decoded = decode_map_square(&files, 947).expect("water patch");
        assert_eq!(1, decoded.water.len());
        assert_eq!(9, decoded.water[0].water_type_id);
        assert_eq!(5, decoded.water[0].tail_parameter);
        assert_eq!(768, decoded.water[0].fine_x_center);
        assert_eq!(-256, decoded.water[0].fine_z_center);
        assert_eq!(1536, decoded.water[0].fine_x_extent);
        assert_eq!(-2048, decoded.water[0].fine_z_extent);
        assert_f32_eq(128.0, decoded.water[0].signed_scalar);
        assert_f32_eq(1.0, decoded.water[0].rotation.x);
        assert!((0.14 - decoded.water[0].tail_transform_x).abs() < f32::EPSILON);
        assert!((-0.16 - decoded.water[0].tail_transform_z).abs() < f32::EPSILON);
    }

    #[test]
    fn decodes_chunk_instance_stream_records() {
        let mut payload = vec![0, 0, 1, 2, 3, 4, 5, 0xaa];
        push_u8(&mut payload, 0x11);
        push_u8(&mut payload, 0);
        push_u8(&mut payload, 1);
        push_u16_be(&mut payload, 42);
        push_u8(&mut payload, 0xfb);
        payload.extend(repeat_n(0xf8, 63));

        let decoded = decode_chunk_instance_stream(&payload).expect("chunk instance stream");

        assert_eq!(1, decoded.chunks.len());
        assert_eq!(1, decoded.records.len());
        assert_eq!(160, decoded.records[0].x);
        assert_eq!(232, decoded.records[0].z);
        assert_eq!(42, decoded.records[0].loc_id);
        assert_eq!(-5, decoded.records[0].info);
    }

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_u16_be(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i16_be(out: &mut Vec<u8>, value: i16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i32_be(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    fn push_f32_be(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }

    fn assert_f32_eq(expected: f32, actual: f32) {
        assert!(
            (expected - actual).abs() < f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn push_zero_u16s(out: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            push_u16_be(out, 0);
        }
    }

    fn push_zero_f32s(out: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            push_f32_be(out, 0.0);
        }
    }
