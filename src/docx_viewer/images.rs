use std::collections::HashMap;

use base64::Engine as _;
use docx_parse::relationships::{
    is_image_relationship, resolve_relationship_target, RelationshipTarget,
};
use docx_parse::s9::S9PackageWire;
use docx_raster::{scoped_image_key, ImageMap, ImageScope};

pub(super) fn load_images(package: &S9PackageWire) -> Result<ImageMap, String> {
    let media = package
        .media_entries
        .iter()
        .map(|(path, media)| (path.to_ascii_lowercase(), media.as_ref()))
        .collect::<HashMap<_, _>>();

    let mut images = ImageMap::new();
    for (id, relationship) in &package.relationship_entries {
        if !is_image_relationship(relationship) {
            continue;
        }
        let RelationshipTarget::Internal(target) =
            resolve_relationship_target("word/_rels/document.xml.rels", relationship)
                .map_err(|error| error.to_string())?
        else {
            continue;
        };
        if let Some(media) = media.get(&target.to_ascii_lowercase()) {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(media.base64.as_bytes())
                .map_err(|_| rust_i18n::t!("docxviewer.image_decode_failed").to_string())?;
            images.insert(scoped_image_key(ImageScope::Body, id), bytes);
        }
    }

    Ok(images)
}
