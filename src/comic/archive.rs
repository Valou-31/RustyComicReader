use anyhow::Result;
use image::ImageReader;
use std::io::Cursor;
use std::path::Path;

pub struct ComicArchive {
    pub pages: Vec<egui::ColorImage>,
}

impl ComicArchive {
    pub async fn load(path: &Path) -> Result<Self> {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();

        let pages = match extension.as_str() {
            "cbz" | "zip" => Self::load_zip(path).await?,
            "cb7" | "7z" => Self::load_7z(path).await?,
            "cbr" | "rar" => Self::load_rar(path).await?,
            _ => anyhow::bail!("Format non supporté: {}", extension),
        };

        Ok(ComicArchive {
            pages: Self::sort_and_filter_images(pages),
        })
    }

    async fn load_zip(path: &Path) -> Result<Vec<(String, egui::ColorImage)>> {  // ✅ Retourne Vec<(String, ColorImage)>
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let mut images = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if Self::is_image_file(&file.name()) {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut data)?;
                if let Ok(img) = Self::decode_image(&data) {
                    images.push((file.name().to_string(), img));
                }
            }
        }

        Ok(images)
    }

    async fn load_7z(_path: &Path) -> Result<Vec<(String, egui::ColorImage)>> {
        todo!("Implémenter 7Z")
    }

    async fn load_rar(_path: &Path) -> Result<Vec<(String, egui::ColorImage)>> {
        todo!("Implémenter RAR")
    }

    fn is_image_file(filename: &str) -> bool {
        let lower = filename.to_lowercase();
        lower.ends_with(".jpg") || lower.ends_with(".jpeg") 
            || lower.ends_with(".png") || lower.ends_with(".webp") 
            || lower.ends_with(".gif")
    }
fn decode_image(data: &[u8]) -> anyhow::Result<egui::ColorImage> {
    // This handles both creation and decoding, returning a Result<DynamicImage, ImageError>
    let img = image::load_from_memory(data)?;
    
    let rgba = img.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        &rgba.into_raw(),
    ))
}

    fn sort_and_filter_images(mut images: Vec<(String, egui::ColorImage)>) -> Vec<egui::ColorImage> {
        images.sort_by(|a, b| {
            alphanumeric_sort::compare_str(&a.0, &b.0)
        });
        images.into_iter().map(|(_, img)| img).collect()
    }
}