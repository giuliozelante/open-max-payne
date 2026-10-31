use maxfx_engine::{
    classify_ldb_lead, count_ras_ldb_leading_vec3, discover_ras_files, list_level_packages,
    looks_like_ldb_or_ai, looks_like_memory_file_ai, looks_like_memory_file_ldb,
    looks_like_ras_ldb, open_archive, parse_levels_txt, parse_ras_ai_preamble, parse_ras_ldb_faces,
    parse_ras_ldb_header, parse_ras_ldb_import_character_head_at, parse_ras_ldb_import_exits_at,
    parse_ras_ldb_import_lightmaps_at, parse_ras_ldb_import_lights_at,
    parse_ras_ldb_import_material_categories_at, parse_ras_ldb_import_materials_at,
    parse_ras_ldb_import_mesh_attribs_at, parse_ras_ldb_import_mesh_polys_at,
    parse_ras_ldb_import_point_stream_at, parse_ras_ldb_import_static_mesh_at,
    parse_ras_ldb_import_textures, parse_ras_ldb_section_at, ras_ldb_after_geometry,
    ras_ldb_after_import_materials, ras_ldb_count_section_end, ras_ldb_first_mf_section_body,
    ras_ldb_import_entity_names_at, ras_ldb_import_jumppoint_names_at, ras_ldb_import_paths,
    ras_ldb_import_stats, ras_ldb_import_tele_names_at, ras_ldb_vec3_section_end,
    soft_walk_mf_tags, split_ldb_records, walk_ras_ldb_known_sections, LDB_IMPORT_VERSION,
    LDB_RAS_MAGIC, LDB_SECTION_KIND_MF, LDB_SECTION_KIND_VEC3, LDB_SECTION_TAG,
};
use openras::RasArchive;
use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: maxfx-engine <archive.ras|archive.mpm|game-dir|level.ldb|.ai>");
        return ExitCode::FAILURE;
    };
    let p = Path::new(&path);
    if p.is_file() {
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("ldb") || ext.eq_ignore_ascii_case("ai") {
            return match std::fs::read(p) {
                Ok(data) => {
                    if looks_like_ras_ldb(&data) {
                        let h = parse_ras_ldb_header(&data).unwrap();
                        let nvec = count_ras_ldb_leading_vec3(&data);
                        let end = ras_ldb_vec3_section_end(&data)
                            .unwrap_or_else(|| ras_ldb_count_section_end(&h));
                        println!(
                            "{}: {} bytes, RAS .ldb magic {:#x} count={} flag={} leading_vec3={} (count_match={})",
                            p.display(),
                            data.len(),
                            h.magic,
                            h.count,
                            h.flag,
                            nvec,
                            h.count as usize == nvec
                        );
                        if let Some(s2) = parse_ras_ldb_section_at(&data, end) {
                            println!(
                                "  section2 @{end}: tag={:#x} kind={:#x} count={} {}",
                                LDB_SECTION_TAG,
                                s2.kind,
                                s2.count,
                                if s2.kind == LDB_SECTION_KIND_MF {
                                    "MF-body"
                                } else {
                                    "other"
                                }
                            );
                        }
                        if let Some(body) = ras_ldb_first_mf_section_body(&data) {
                            let faces = parse_ras_ldb_faces(body, 3);
                            if let Some(f) = faces.first() {
                                println!(
                                    "  face0 indices={:?} attr={} n=({:.3},{:.3},{:.3}) p=({:.3},{:.3},{:.3})",
                                    f.indices,
                                    f.attr,
                                    f.normal[0],
                                    f.normal[1],
                                    f.normal[2],
                                    f.point[0],
                                    f.point[1],
                                    f.point[2]
                                );
                            }
                            println!("  parsed {} face prefix(es)", faces.len());
                        }
                        let spans = walk_ras_ldb_known_sections(&data);
                        println!("  known sections ({}):", spans.len());
                        for s in &spans {
                            let label = match s.kind {
                                LDB_SECTION_KIND_VEC3 => "vec3",
                                LDB_SECTION_KIND_MF if s.end > s.offset => "mf-ok",
                                LDB_SECTION_KIND_MF => "mf-stop",
                                _ => "other",
                            };
                            println!(
                                "    @{:#x} kind={:#x} ({label}) count={} end={:#x}",
                                s.offset, s.kind, s.count, s.end
                            );
                        }
                        match ras_ldb_after_geometry(&data) {
                            Some(at) => {
                                println!(
                                    "  ImportLevel MF @ {at:#x} (14 {:#x})",
                                    LDB_IMPORT_VERSION
                                );
                                if let Some((np, nj)) = ras_ldb_import_stats(&data) {
                                    println!("  import stream: {np} paths, {nj} JPEG SOIs");
                                }
                                if let Some((tex_count, texs, tex_end)) =
                                    parse_ras_ldb_import_textures(&data, 5)
                                {
                                    println!(
                                        "  import textures: declared={tex_count} shown={} end={tex_end:#x}",
                                        texs.len()
                                    );
                                    for t in &texs {
                                        println!(
                                            "    {} flag={} blob={} jpeg={}",
                                            t.path, t.flag, t.blob_len, t.jpeg
                                        );
                                    }
                                    let mut mat_off = tex_end;
                                    let mut mat_pass = 0;
                                    while let Some((mc, mats, mend)) =
                                        parse_ras_ldb_import_materials_at(&data, mat_off, 5)
                                    {
                                        mat_pass += 1;
                                        println!(
                                            "  import materials[{mat_pass}] @{mat_off:#x}: declared={mc} shown={} end={mend:#x}",
                                            mats.len()
                                        );
                                        for m in &mats {
                                            println!(
                                                "    {} → {} (#{})",
                                                m.name, m.texture, m.index
                                            );
                                        }
                                        if mend <= mat_off {
                                            break;
                                        }
                                        mat_off = mend;
                                        if mat_pass >= 4 {
                                            break;
                                        }
                                    }
                                    if let Some(cat_off) = ras_ldb_after_import_materials(&data) {
                                        if let Some((cc, cats, cend)) =
                                            parse_ras_ldb_import_material_categories_at(
                                                &data, cat_off,
                                            )
                                        {
                                            let members: usize =
                                                cats.iter().map(|c| c.members.len()).sum();
                                            println!(
                                                "  import material categories @{cat_off:#x}: declared={cc} shown={} members={members} end={cend:#x}",
                                                cats.len()
                                            );
                                            for c in cats.iter().take(5) {
                                                println!(
                                                    "    {} members={} tex={} flags={}/{}",
                                                    c.name,
                                                    c.member_count,
                                                    c.texture,
                                                    c.flag_a,
                                                    c.flag_b
                                                );
                                            }
                                            if let Some((lc, lend)) =
                                                parse_ras_ldb_import_lightmaps_at(&data, cend)
                                            {
                                                println!(
                                                    "  import lightmaps @{cend:#x}: count={lc} end={lend:#x}"
                                                );
                                                if let Some((exits, eend)) =
                                                    parse_ras_ldb_import_exits_at(&data, lend, 64)
                                                {
                                                    let with_pos = exits
                                                        .iter()
                                                        .filter(|e| e.pos.is_some())
                                                        .count();
                                                    println!(
                                                        "  import exits @{lend:#x}: shown={} with_room={with_pos} end={eend:#x}",
                                                        exits.len()
                                                    );
                                                    for e in exits.iter().take(6) {
                                                        match e.pos {
                                                            Some([x, y, z]) => println!(
                                                                "    {} id={} room@({x:.1},{y:.1},{z:.1})",
                                                                e.name, e.id
                                                            ),
                                                            None => println!(
                                                                "    {} id={} (link)",
                                                                e.name, e.id
                                                            ),
                                                        }
                                                    }
                                                    if let Some((pn, pend)) =
                                                        parse_ras_ldb_import_point_stream_at(
                                                            &data, eend,
                                                        )
                                                    {
                                                        println!(
                                                            "  import point stream @{eend:#x}: bodies={pn} end={pend:#x}"
                                                        );
                                                        let mut mesh_off = pend;
                                                        let mut mesh_n = 0;
                                                        while let Some((mesh, mend)) =
                                                            parse_ras_ldb_import_static_mesh_at(
                                                                &data, mesh_off,
                                                            )
                                                        {
                                                            mesh_n += 1;
                                                            let [x, y, z] = mesh.pos;
                                                            println!(
                                                                    "  import static mesh[{mesh_n}] @{mesh_off:#x}: a={} b={} points={} pos=({x:.1},{y:.1},{z:.1}) end={mend:#x}",
                                                                    mesh.a, mesh.b, mesh.point_count
                                                                );
                                                            let mut next = mend;
                                                            if let Some((h0, polys, pend2)) =
                                                                parse_ras_ldb_import_mesh_polys_at(
                                                                    &data, mend,
                                                                )
                                                            {
                                                                println!(
                                                                        "  import mesh polys @{mend:#x}: hdr={h0} shown={} end={pend2:#x}",
                                                                        polys.len()
                                                                    );
                                                                next = pend2;
                                                            }
                                                            if let Some((ac, attrs, aend)) =
                                                                parse_ras_ldb_import_mesh_attribs_at(
                                                                    &data, next,
                                                                )
                                                            {
                                                                println!(
                                                                        "  import mesh attribs @{next:#x}: count={ac} shown={} end={aend:#x}",
                                                                        attrs.len()
                                                                    );
                                                                next = aend;
                                                            }
                                                            if next <= mesh_off {
                                                                break;
                                                            }
                                                            mesh_off = next;
                                                            if mesh_n >= 32 {
                                                                break;
                                                            }
                                                        }
                                                        if let Some((cc, cstart)) =
                                                            parse_ras_ldb_import_character_head_at(
                                                                &data, mesh_off,
                                                            )
                                                        {
                                                            let jumps =
                                                                ras_ldb_import_jumppoint_names_at(
                                                                    &data, cstart, 16,
                                                                );
                                                            let ents =
                                                                ras_ldb_import_entity_names_at(
                                                                    &data, cstart, 48,
                                                                );
                                                            let (tags, wend) = soft_walk_mf_tags(
                                                                &data, mesh_off, 500_000,
                                                            );
                                                            println!(
                                                                    "  import character head @{mesh_off:#x}: count={cc} first_name@{cstart:#x} jumppoints={} entities={} soft_tags={tags} walk_end={wend:#x}/{:#x}",
                                                                    jumps.len(),
                                                                    ents.len(),
                                                                    data.len()
                                                                );
                                                            for jn in &jumps {
                                                                println!("    jump {jn}");
                                                            }
                                                            for en in ents.iter().take(12) {
                                                                println!("    ent {en}");
                                                            }
                                                        } else if let Some((lc, lights, lend)) =
                                                            parse_ras_ldb_import_lights_at(
                                                                &data, mesh_off, 8,
                                                            )
                                                        {
                                                            println!(
                                                                    "  import lights @{mesh_off:#x}: count={lc} shown={} end={lend:#x}",
                                                                    lights.len()
                                                                );
                                                            for lit in &lights {
                                                                let [x, y, z] = lit.pos;
                                                                println!(
                                                                        "    light {} subtype={} params={} pos=({x:.1},{y:.1},{z:.1})",
                                                                        lit.name, lit.subtype, lit.param_count
                                                                    );
                                                            }
                                                            let teles =
                                                                ras_ldb_import_tele_names_at(
                                                                    &data, lend, 24,
                                                                );
                                                            if !teles.is_empty() {
                                                                println!(
                                                                        "  import teles after lights: shown={}",
                                                                        teles.len()
                                                                    );
                                                                for t in &teles {
                                                                    println!("    tele {t}");
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    let paths = ras_ldb_import_paths(&data, 8);
                                    for pth in &paths {
                                        println!("    path {pth}");
                                    }
                                }
                            }
                            None => println!("  ImportLevel MF head not found after geometry"),
                        }
                        return ExitCode::SUCCESS;
                    }
                    if looks_like_memory_file_ldb(&data) {
                        println!(
                            "{}: {} bytes, MemoryFile-native ImportLevel (version tag {:#x})",
                            p.display(),
                            data.len(),
                            LDB_IMPORT_VERSION
                        );
                        return ExitCode::SUCCESS;
                    }
                    if looks_like_memory_file_ai(&data) {
                        print!(
                            "{}: {} bytes, MemoryFile-tagged .ai (head 0x14 {:#x})",
                            p.display(),
                            data.len(),
                            data[1]
                        );
                        if let Some(h) = maxfx_engine::parse_ras_ai_header(&data) {
                            println!(" ints=[{},{},{},{}]", h.a, h.b, h.c, h.d);
                        } else {
                            println!();
                        }
                        if let Some((pend, grav)) = parse_ras_ai_preamble(&data) {
                            let [gx, gy, gz] = grav;
                            let (n, wend) = soft_walk_mf_tags(&data, pend, 500_000);
                            println!(
                                "  ai preamble end={pend:#x} gravity=({gx:.1},{gy:.1},{gz:.1}) soft_tags={n} walk_end={wend:#x}/{:#x}{}",
                                data.len(),
                                if wend == data.len() { " (EOF)" } else { "" }
                            );
                        }
                        return ExitCode::SUCCESS;
                    }
                    if !looks_like_ldb_or_ai(&data) {
                        eprintln!(
                            "error: unrecognized level blob (want RAS {:#x}, MF 14 {:#x}, or legacy space/>)",
                            LDB_RAS_MAGIC, LDB_IMPORT_VERSION
                        );
                        return ExitCode::FAILURE;
                    }
                    let recs = split_ldb_records(&data).unwrap();
                    println!(
                        "{}: {} bytes, {} >-records after legacy preamble",
                        p.display(),
                        data.len(),
                        recs.len()
                    );
                    let mut leads = std::collections::BTreeMap::<u8, usize>::new();
                    for r in &recs {
                        if let Some(b) = r.lead_byte() {
                            *leads.entry(b).or_default() += 1;
                        }
                    }
                    println!("lead bytes (top):");
                    let mut pairs: Vec<_> = leads.into_iter().collect();
                    pairs.sort_by_key(|a| std::cmp::Reverse(a.1));
                    for (b, n) in pairs.into_iter().take(12) {
                        println!("  0x{b:02x} ({}): {n}", classify_ldb_lead(b));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            };
        }
    }
    if p.is_dir() {
        return match discover_ras_files(p) {
            Ok(files) => {
                println!("{}: {} *.ras", p.display(), files.len());
                for f in files {
                    match open_archive(&f) {
                        Ok(a) => println!(
                            "  {}: {} files, ver={} compat={}",
                            f.file_name().unwrap_or_default().to_string_lossy(),
                            a.file_count,
                            a.version,
                            a.compatibility
                        ),
                        Err(e) => println!("  {}: error: {e}", f.display()),
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match RasArchive::read(p) {
        Ok(arc) => {
            match open_archive(p) {
                Ok(a) => println!(
                    "{}: {} files, {} dirs, ver={} compat={}",
                    a.label, a.file_count, a.dir_count, a.version, a.compatibility
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
            let pkgs = list_level_packages(&arc);
            if !pkgs.is_empty() {
                println!("level packages ({}):", pkgs.len());
                for pkg in pkgs {
                    match &pkg.ai {
                        Some(ai) => println!("  {} + {}", pkg.ldb, ai),
                        None => println!("  {} (no .ai)", pkg.ldb),
                    }
                }
            }
            if let Some(txt) = extract_levels_txt(&arc) {
                let entries = parse_levels_txt(&txt);
                println!("levels.txt entries ({}):", entries.len());
                for e in entries.iter().take(12) {
                    println!("  {} → {}", e.id, e.ldb_archive_path());
                }
                if entries.len() > 12 {
                    println!("  … {} more", entries.len() - 12);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn extract_levels_txt(arc: &RasArchive) -> Option<String> {
    const WANT: &str = "\\data\\database\\levels\\levels.txt";
    for f in &arc.files {
        let dname = arc
            .directories
            .get(f.dir_index as usize)
            .map(|d| d.name.as_str())
            .unwrap_or("\\");
        let full = format!("{dname}{}", f.name);
        if full.eq_ignore_ascii_case(WANT) {
            return Some(String::from_utf8_lossy(&f.data).into_owned());
        }
    }
    None
}
