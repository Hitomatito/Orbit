use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use gio::ListStore;

use crate::db::Database;
use crate::discovery::AppDiscoveryEngine;
use crate::models::AppFootprint;
use crate::rt::AsyncRuntime;

thread_local! {
    static APP_STORE: RefCell<Option<ListStore>> = RefCell::new(None);
    static SCAN_BTN: RefCell<Option<gtk::Button>> = RefCell::new(None);
}

struct SharedState {
    rt: AsyncRuntime,
    discovery: AppDiscoveryEngine,
    apps: Mutex<Vec<AppFootprint>>,
}

pub struct OrbitApp {
    app: adw::Application,
}

impl OrbitApp {
    pub fn new(rt: AsyncRuntime, db: Arc<Database>) -> Self {
        let app = adw::Application::builder()
            .application_id("com.orbit.AppManager")
            .build();

        let discovery = AppDiscoveryEngine::new(db);
        let state = Arc::new(SharedState {
            rt,
            discovery,
            apps: Mutex::new(Vec::new()),
        });

        app.connect_activate(move |app| {
            Self::build_ui(app, state.clone());
        });

        Self { app }
    }

    pub fn run(&self) {
        self.app.run();
    }

    fn build_ui(app: &adw::Application, state: Arc<SharedState>) {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Orbit")
            .default_width(960)
            .default_height(720)
            .build();

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();

        let scan_button = gtk::Button::with_label("Scan");
        scan_button.add_css_class("suggested-action");
        header.pack_start(&scan_button);

        let store = ListStore::new::<gtk::StringObject>();
        let list_view = create_app_list(&store);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list_view)
            .vexpand(true)
            .build();

        APP_STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
        SCAN_BTN.with(|b| *b.borrow_mut() = Some(scan_button.clone()));

        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scrolled));

        window.set_content(Some(&toolbar));

        scan_button.connect_clicked(move |_| {
            SCAN_BTN.with(|b| {
                if let Some(ref btn) = *b.borrow() {
                    btn.set_sensitive(false);
                    btn.set_label("Scanning…");
                }
            });

            let state_for_task = state.clone();
            let (tx, rx) = mpsc::channel::<usize>();

            state.rt.spawn_task(async move {
                let cancel = tokio_util::sync::CancellationToken::new();
                let result = state_for_task.discovery.scan_all(cancel).await;
                *state_for_task.apps.lock().unwrap() = state_for_task.discovery.get_cached_apps();
                let _ = tx.send(result.total);
            });

            let state_for_ui = state.clone();
            glib::idle_add(move || match rx.try_recv() {
                Ok(total) => {
                    APP_STORE.with(|s| {
                        if let Some(ref store) = *s.borrow() {
                            store.remove_all();
                            for app in state_for_ui.apps.lock().unwrap().iter() {
                                store.append(&gtk::StringObject::new(&app.display_name));
                            }
                        }
                    });
                    SCAN_BTN.with(|b| {
                        if let Some(ref btn) = *b.borrow() {
                            btn.set_sensitive(true);
                            btn.set_label(&format!("Scan ({} apps)", total));
                        }
                    });
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            });
        });

        window.present();
    }
}

fn create_app_list(store: &ListStore) -> gtk::ListView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(move |_factory, item| {
        let list_item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_margin_top(6);
        label.set_margin_bottom(6);
        label.add_css_class("body");
        list_item.set_child(Some(&label));
    });

    factory.connect_bind(move |_factory, item| {
        let list_item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let label = list_item
            .child()
            .and_downcast::<gtk::Label>()
            .expect("Label");
        let string_obj = list_item
            .item()
            .and_downcast::<gtk::StringObject>()
            .expect("StringObject");
        label.set_label(&string_obj.string());
    });

    let selection = gtk::SingleSelection::new(Some(store.clone()));
    gtk::ListView::new(Some(selection), Some(factory))
}
