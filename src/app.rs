use adw::prelude::*;

pub struct OrbitApp {
    app: adw::Application,
}

impl OrbitApp {
    pub fn new() -> Self {
        let app = adw::Application::builder()
            .application_id("com.orbit.AppManager")
            .build();

        app.connect_activate(move |app| {
            Self::build_ui(app);
        });

        Self { app }
    }

    pub fn run(&self) {
        self.app.run();
    }

    fn build_ui(app: &adw::Application) {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Orbit")
            .default_width(960)
            .default_height(720)
            .build();

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();

        let label = gtk::Label::builder()
            .label("Orbit — Application Footprint Manager")
            .css_classes(["title-1"])
            .margin_top(48)
            .build();

        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&label));

        window.set_content(Some(&toolbar));
        window.present();
    }
}
