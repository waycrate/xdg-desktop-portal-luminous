use ashpd::desktop::file_chooser::{Choice, FileFilter, OpenFileRequest, SaveFileRequest};

#[async_std::main]
async fn main() -> ashpd::Result<()> {
    let files = OpenFileRequest::default()
        .title("open a file to read")
        .accept_label("read")
        .modal(true)
        .multiple(true)
        .choice(
            Choice::new("encoding", "Encoding", "latin15")
                .insert("utf8", "Unicode (UTF-8)")
                .insert("latin15", "Western"),
        )
        // A trick to have a checkbox
        .choice(Choice::boolean("re-encode", "Re-encode", false))
        .filter(FileFilter::new("SVG Image").mimetype("image/svg+xml"))
        .filter(FileFilter::new("JPEG Image").glob("*.jpg"))
        .current_filter(FileFilter::new("JPEG Image").glob("*.jpg"))
        .send()
        .await?
        .response()?;

    println!("{:#?}", files);

    let files = SaveFileRequest::default()
        .title("open a file to write")
        .accept_label("write")
        .current_name("image.jpg")
        .modal(true)
        //.filter(FileFilter::new("SVG Image").mimetype("image/svg+xml"))
        .filter(FileFilter::new("JPEG Image").glob("*.jpg"))
        .current_filter(FileFilter::new("JPEG Image").glob("*.jpg"))
        .send()
        .await?
        .response()?;

    println!("{:#?}", files);


    Ok(())
}
