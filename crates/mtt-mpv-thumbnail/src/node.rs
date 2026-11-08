//! Borrowed views: only use while the originating libmpv node/event remains alive.
use crate::ffi::Node;
use anyhow::{bail, ensure, Result};
use serde_json::{Map, Value};
use std::ffi::CStr;

impl Node {
    pub fn integer(&self) -> Result<i64> {
        ensure!(self.format == 4, "Expected an integer node");
        // SAFETY: the discriminant has been checked.
        Ok(unsafe { self.value.integer })
    }

    pub fn string(&self) -> Result<&str> {
        ensure!(self.format == 1, "Expected a string node");
        unsafe {
            ensure!(!self.value.string.is_null(), "Null string node");
            Ok(CStr::from_ptr(self.value.string).to_str()?)
        }
    }

    pub fn bytes(&self) -> Result<&[u8]> {
        ensure!(self.format == 9, "Expected a byte array node");
        unsafe {
            let bytes = self
                .value
                .bytes
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Null byte array"))?;
            ensure!(bytes.size <= isize::MAX as usize, "Byte array is too large");
            if bytes.size == 0 {
                return Ok(&[]);
            }
            ensure!(!bytes.data.is_null(), "Null pixel buffer");
            Ok(std::slice::from_raw_parts(bytes.data.cast(), bytes.size))
        }
    }

    pub fn entries(&self) -> Result<(&[Node], Option<&[*mut std::ffi::c_char]>)> {
        ensure!(matches!(self.format, 7 | 8), "Expected a list node");
        unsafe {
            let list = self
                .value
                .list
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Null node list"))?;
            ensure!((0..=1_000_000).contains(&list.count), "Invalid node count");
            if list.count == 0 {
                return Ok((&[], (self.format == 8).then_some(&[])));
            }
            ensure!(!list.values.is_null(), "Null list values");
            let values = std::slice::from_raw_parts(list.values, list.count as usize);
            let keys = if self.format == 8 {
                ensure!(!list.keys.is_null(), "Null map keys");
                Some(std::slice::from_raw_parts(list.keys, list.count as usize))
            } else {
                None
            };
            Ok((values, keys))
        }
    }

    pub fn field(&self, name: &str) -> Result<&Node> {
        let (values, keys) = self.entries()?;
        let keys = keys.ok_or_else(|| anyhow::anyhow!("Expected a map"))?;
        for (value, &key) in values.iter().zip(keys) {
            ensure!(!key.is_null(), "Null map key");
            if unsafe { CStr::from_ptr(key) }.to_bytes() == name.as_bytes() {
                return Ok(value);
            }
        }
        bail!("Missing screenshot field: {name}")
    }

    pub fn json(&self) -> Result<Value> {
        Ok(match self.format {
            0 => Value::Null,
            1 => self.string()?.into(),
            3 => unsafe { self.value.flag != 0 }.into(),
            4 => self.integer()?.into(),
            5 => unsafe { self.value.double }.into(),
            7 | 8 => {
                let (values, keys) = self.entries()?;
                if let Some(keys) = keys {
                    let mut map = Map::new();
                    for (node, &key) in values.iter().zip(keys) {
                        ensure!(!key.is_null(), "Null map key");
                        let key = unsafe { CStr::from_ptr(key) }.to_str()?;
                        map.insert(key.into(), node.json()?);
                    }
                    Value::Object(map)
                } else {
                    Value::Array(values.iter().map(Node::json).collect::<Result<_>>()?)
                }
            }
            _ => bail!("Unsupported node format: {}", self.format),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{ByteArray, NodeList, Value as NodeValue};
    use std::ffi::CString;

    #[test]
    fn borrows_map_values_and_serializes_inventory() {
        let key = CString::new("codec").unwrap();
        let value = CString::new("h264").unwrap();
        let mut values = [Node {
            format: 1,
            value: NodeValue {
                string: value.as_ptr().cast_mut(),
            },
        }];
        let mut keys = [key.as_ptr().cast_mut()];
        let mut list = NodeList {
            count: 1,
            values: values.as_mut_ptr(),
            keys: keys.as_mut_ptr(),
        };
        let map = Node {
            format: 8,
            value: NodeValue { list: &mut list },
        };
        assert_eq!(map.field("codec").unwrap().string().unwrap(), "h264");
        assert_eq!(map.json().unwrap(), serde_json::json!({"codec": "h264"}));
        assert!(map.field("missing").is_err());
    }

    #[test]
    fn rejects_wrong_types_and_null_payloads() {
        let number = Node {
            format: 4,
            value: NodeValue { integer: 42 },
        };
        assert_eq!(number.integer().unwrap(), 42);
        assert!(number.string().is_err());
        assert!(number.bytes().is_err());
        let string = Node {
            format: 1,
            value: NodeValue {
                string: std::ptr::null_mut(),
            },
        };
        assert!(string.string().is_err());
        let mut bytes = ByteArray {
            size: 1,
            data: std::ptr::null_mut(),
        };
        let array = Node {
            format: 9,
            value: NodeValue { bytes: &mut bytes },
        };
        assert!(array.bytes().is_err());
    }

    #[test]
    fn accepts_empty_lists_without_dereferencing_null_entries() {
        let mut list = NodeList {
            count: 0,
            values: std::ptr::null_mut(),
            keys: std::ptr::null_mut(),
        };
        let array = Node {
            format: 7,
            value: NodeValue { list: &mut list },
        };
        assert_eq!(array.json().unwrap(), serde_json::json!([]));
        let map = Node {
            format: 8,
            value: NodeValue { list: &mut list },
        };
        assert_eq!(map.json().unwrap(), serde_json::json!({}));
    }
}
