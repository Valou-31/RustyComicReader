use anyhow::Result;
use std::path::Path;

pub struct ComicArchive {
    pub pages: Vec<egui::ColorImage>,
}

/// Called after each page is decoded, in archive order (not final reading
/// order — that's only known once every entry has been read and sorted).
type ProgressFn<'a> = dyn FnMut(usize, usize, &egui::ColorImage) + 'a;

impl ComicArchive {
    pub async fn load(path: &Path, mut on_progress: impl FnMut(usize, usize, &egui::ColorImage)) -> Result<Self> {
        let extension = path.extension().unwrap_or_default().to_str().unwrap_or("").to_lowercase();

        let pages = match extension.as_str() {
            "cbz" | "zip" => Self::load_zip(path, &mut on_progress).await?,
            "cb7" | "7z" => Self::load_7z(path, &mut on_progress).await?,
            "cbr" | "rar" => Self::load_rar(path, &mut on_progress).await?,
            _ => anyhow::bail!("Format non supporté: {}", extension),
        };

        Ok(ComicArchive {
            pages: Self::sort_and_filter_images(pages),
        })
    }

    async fn load_zip(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, egui::ColorImage)>> {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let total = archive.file_names().filter(|n| Self::is_image_file(n)).count();
        let mut images = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if Self::is_image_file(&file.name()) {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut data)?;
                if let Ok(img) = Self::decode_image(&data) {
                    images.push((file.name().to_string(), img));
                    on_progress(images.len(), total, &images.last().unwrap().1);
                }
            }
        }

        Ok(images)
    }

    async fn load_7z(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, egui::ColorImage)>> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let mut archive = sevenz_rust::SevenZReader::new(file, len, sevenz_rust::Password::empty())
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        let total = archive
            .archive()
            .files
            .iter()
            .filter(|entry| !entry.is_directory() && Self::is_image_file(entry.name()))
            .count();

        let mut images = Vec::new();
        archive
            .for_each_entries(|entry, reader| {
                if !entry.is_directory() && Self::is_image_file(entry.name()) {
                    let mut data = Vec::new();
                    reader.read_to_end(&mut data)?;
                    if let Ok(img) = Self::decode_image(&data) {
                        images.push((entry.name().to_string(), img));
                        on_progress(images.len(), total, &images.last().unwrap().1);
                    }
                }
                Ok(true)
            })
            .map_err(|e| anyhow::anyhow!("Erreur lecture 7z: {e}"))?;

        Ok(images)
    }

    async fn load_rar(path: &Path, on_progress: &mut ProgressFn<'_>) -> Result<Vec<(String, egui::ColorImage)>> {
        let total = unrar::Archive::new(path)
            .open_for_listing()
            .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| !entry.is_directory() && Self::is_image_file(&entry.filename.to_string_lossy()))
            .count();

        let mut images = Vec::new();

        let archive = unrar::Archive::new(path)
            .open_for_processing()
            .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;

        let mut cursor = Some(archive);
        while let Some(current) = cursor.take() {
            let Some(file) = current
                .read_header()
                .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?
            else {
                break;
            };

            let name = file.entry().filename.to_string_lossy().into_owned();
            let is_image = !file.entry().is_directory() && Self::is_image_file(&name);

            if is_image {
                let (data, next) = file
                    .read()
                    .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?;
                if let Ok(img) = Self::decode_image(&data) {
                    images.push((name, img));
                    on_progress(images.len(), total, &images.last().unwrap().1);
                }
                cursor = Some(next);
            } else {
                cursor = Some(
                    file.skip()
                        .map_err(|e| anyhow::anyhow!("Erreur lecture RAR: {e}"))?,
                );
            }
        }

        Ok(images)
    }

    fn is_image_file(filename: &str) -> bool {
        let lower = filename.to_lowercase();
        lower.ends_with(".jpg") || lower.ends_with(".jpeg")
            || lower.ends_with(".png") || lower.ends_with(".webp")
            || lower.ends_with(".gif")
    }

    fn decode_image(data: &[u8]) -> anyhow::Result<egui::ColorImage> {
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
