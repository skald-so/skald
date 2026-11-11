//! The test vectors in `docs/protocol/test-vectors.md`: `proto` encodes each value to exactly
//! the published bytes, decodes them back, and the published diagnostic notation describes them.

mod common;

use common::{Vector, hex, to_hex, vectors};

const DOC: &str = include_str!("../../../docs/protocol/test-vectors.md");

/// A vector as the document publishes it.
struct Published {
    name: String,
    diag: String,
    hex: String,
}

/// The vectors of the document: each `##` section but "Contents", with its `cbor-diag` and `hex`
/// blocks.
fn published() -> Vec<Published> {
    let mut found = Vec::new();
    let mut lines = DOC.lines();
    while let Some(line) = lines.next() {
        if let Some(name) = line.strip_prefix("## ") {
            if name != "Contents" {
                found.push(Published {
                    name: name.to_owned(),
                    diag: String::new(),
                    hex: String::new(),
                });
            }
            continue;
        }
        let block: fn(&mut Published) -> &mut String = match line {
            "```cbor-diag" => |vector: &mut Published| &mut vector.diag,
            "```hex" => |vector: &mut Published| &mut vector.hex,
            _ => continue,
        };
        let vector = found
            .last_mut()
            .expect("a code block before the first vector");
        let text = block(vector);
        for line in lines.by_ref().take_while(|line| *line != "```") {
            text.push_str(line);
            text.push('\n');
        }
    }
    found
}

/// The section the document should have for `vector`.
fn section(vector: &Vector) -> String {
    let hex = to_hex(&vector.bytes);
    let lines: Vec<&str> = hex
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    format!(
        "## {}\n\n```cbor-diag\n{}\n```\n\n```hex\n{}\n```\n",
        vector.name,
        diag(&vector.bytes).unwrap(),
        lines.join("\n")
    )
}

#[test]
fn the_document_lists_every_vector_in_order() {
    let published: Vec<String> = published().into_iter().map(|vector| vector.name).collect();
    let expected: Vec<&str> = vectors().iter().map(|vector| vector.name).collect();
    assert_eq!(published, expected);
}

#[test]
fn proto_output_matches_the_vectors_byte_for_byte() {
    let published = published();
    for vector in vectors() {
        let doc = published
            .iter()
            .find(|doc| doc.name == vector.name)
            .unwrap_or_else(|| panic!("no vector {:?}; add:\n\n{}", vector.name, section(&vector)));
        assert_eq!(
            to_hex(&hex(&doc.hex)),
            to_hex(&vector.bytes),
            "{}; expected:\n\n{}",
            vector.name,
            section(&vector)
        );
        assert_eq!(
            (vector.decode)(&hex(&doc.hex)).as_ref(),
            Ok(&vector.bytes),
            "{}",
            vector.name
        );
        assert_eq!(
            doc.diag.trim_end(),
            diag(&vector.bytes).unwrap(),
            "{}; expected:\n\n{}",
            vector.name,
            section(&vector)
        );
    }
}

/// The diagnostic notation of one CBOR item (RFC 8949, §8), as the document writes it: two-space
/// indents, and `<< >>` (RFC 8610, Appendix G.3) around a byte string that holds an array or a
/// map. Only the forms the spec allows; `None` for anything else.
fn diag(bytes: &[u8]) -> Option<String> {
    let mut at = 0;
    let text = item(bytes, &mut at, "")?;
    (at == bytes.len()).then_some(text)
}

fn item(bytes: &[u8], at: &mut usize, indent: &str) -> Option<String> {
    let initial = *bytes.get(*at)?;
    *at += 1;
    let argument = match initial & 0x1f {
        info @ 0..24 => u64::from(info),
        info @ 24..28 => {
            let len = 1 << (info - 24);
            let mut value = 0;
            for &byte in bytes.get(*at..*at + len)? {
                value = value << 8 | u64::from(byte);
            }
            *at += len;
            value
        }
        _ => return None,
    };
    let len = usize::try_from(argument).ok();
    let inner = format!("{indent}  ");
    Some(match initial >> 5 {
        0 => argument.to_string(),
        1 => (-1 - i128::from(argument)).to_string(),
        2 => {
            let content = bytes.get(*at..*at + len?)?;
            *at += len?;
            match content.first().map(|first| first >> 5) {
                Some(4 | 5) if diag(content).is_some() => {
                    let mut nested = 0;
                    format!("<<{}>>", item(content, &mut nested, indent)?)
                }
                _ => format!("h'{}'", to_hex(content)),
            }
        }
        3 => {
            let content = std::str::from_utf8(bytes.get(*at..*at + len?)?).ok()?;
            *at += len?;
            let escaped: String = content
                .chars()
                .flat_map(|char| match char {
                    '"' | '\\' => vec!['\\', char],
                    _ => vec![char],
                })
                .collect();
            format!("\"{escaped}\"")
        }
        4 if argument == 0 => "[]".to_owned(),
        5 if argument == 0 => "{}".to_owned(),
        4 => {
            let mut items = Vec::new();
            for _ in 0..argument {
                items.push(format!("{inner}{}", item(bytes, at, &inner)?));
            }
            format!("[\n{}\n{indent}]", items.join(",\n"))
        }
        5 => {
            let mut entries = Vec::new();
            for _ in 0..argument {
                let key = item(bytes, at, &inner)?;
                let value = item(bytes, at, &inner)?;
                entries.push(format!("{inner}{key}: {value}"));
            }
            format!("{{\n{}\n{indent}}}", entries.join(",\n"))
        }
        7 => match argument {
            20 => "false".to_owned(),
            21 => "true".to_owned(),
            22 => "null".to_owned(),
            _ => return None,
        },
        _ => return None,
    })
}
