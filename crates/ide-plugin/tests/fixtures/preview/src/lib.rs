//! Test fixture for the ide-plugin preview mechanism. Never shipped.
//!
//! `render-preview` echoes the request into a small HTML document. Content
//! starting with `!` makes it return an error; content containing `dark` asks
//! for a dark background.

wit_bindgen::generate!({
    world: "plugin-preview",
    path: "../../../../../wit",
});

struct PreviewFixture;

impl Guest for PreviewFixture {
    fn activate(_ctx: Context) -> Result<(), String> {
        Ok(())
    }

    fn deactivate() {}

    fn run_command(id: String, arg: Option<String>) -> Result<Option<String>, String> {
        match id.as_str() {
            "echo" => Ok(arg),
            other => Err(format!("unknown command: {other}")),
        }
    }

    fn render_preview(request: PreviewRequest) -> Result<PreviewOutput, String> {
        if let Some(text) = request.content.strip_prefix('!') {
            return Err(format!("preview refused: {text}"));
        }
        let background = request.content.contains("dark").then(|| "#111111".to_owned());
        let html = format!(
            "<main data-decl=\"{}\"><h1>{}</h1><p>{}</p><pre>{}</pre></main>",
            request.decl,
            request.decl,
            request.project_path,
            request.content.replace('<', "&lt;")
        );
        Ok(PreviewOutput { html, background })
    }
}

export!(PreviewFixture);
