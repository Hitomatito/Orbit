use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use gio::ListStore;

use crate::adapters::AdapterError;
use crate::db::Database;
use crate::discovery::AppDiscoveryEngine;
use crate::models::{AppFootprint, PackageSource};
use crate::rt::AsyncRuntime;
use tokio_util::sync::CancellationToken;

thread_local! {
    static SCAN_BTN: RefCell<Option<gtk::Button>> = RefCell::new(None);
    static SCAN_SPINNER: RefCell<Option<gtk::Spinner>> = RefCell::new(None);
    static NAV_VIEW: RefCell<Option<adw::NavigationView>> = RefCell::new(None);
    static APP_STORE: RefCell<Option<gio::ListStore>> = RefCell::new(None);
    static SORT_MODEL: RefCell<Option<gtk::SortListModel>> = RefCell::new(None);
    static SORT_DROPDOWN: RefCell<Option<gtk::DropDown>> = RefCell::new(None);
    static APP_WINDOW: RefCell<Option<adw::ApplicationWindow>> = RefCell::new(None);
}

struct SharedState {
    rt: AsyncRuntime,
    discovery: AppDiscoveryEngine,
    apps: Mutex<Vec<AppFootprint>>,
    db: Arc<Database>,
}

pub struct OrbitApp {
    app: adw::Application,
}

impl OrbitApp {
    pub fn new(rt: AsyncRuntime, db: Arc<Database>) -> Self {
        let app = adw::Application::builder()
            .application_id("com.orbit.AppManager")
            .build();

        let discovery = AppDiscoveryEngine::new(db.clone());
        let state = Arc::new(SharedState {
            rt,
            discovery,
            apps: Mutex::new(Vec::new()),
            db,
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

        APP_WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));

        let nav = adw::NavigationView::new();
        let list_page = build_list_page(state.clone());
        nav.push(&list_page);

        NAV_VIEW.with(|n| *n.borrow_mut() = Some(nav.clone()));

        load_cached_apps(state);

        window.set_content(Some(&nav));
        window.present();
    }
}

fn build_list_page(state: Arc<SharedState>) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let scan_btn = gtk::Button::with_label("Scan");
    scan_btn.add_css_class("suggested-action");
    header.pack_start(&scan_btn);

    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    header.pack_start(&spinner);

    SCAN_SPINNER.with(|s| *s.borrow_mut() = Some(spinner));

    let sort_options = gtk::StringList::new(&[
        "Name (A-Z)",
        "Name (Z-A)",
        "Size",
        "Source",
    ]);
    let sort_dropdown = gtk::DropDown::new(Some(sort_options), None::<&gtk::PropertyExpression>);
    sort_dropdown.set_selected(0);
    SORT_DROPDOWN.with(|s| *s.borrow_mut() = Some(sort_dropdown.clone()));
    header.pack_end(&sort_dropdown);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search apps…")
        .width_request(240)
        .build();
    header.pack_end(&search_entry);

    let store = ListStore::new::<gtk::StringObject>();
    APP_STORE.with(|s| *s.borrow_mut() = Some(store.clone()));

    let filter_model = gtk::FilterListModel::new(Some(store.clone()), None::<gtk::CustomFilter>);
    let sort_model = gtk::SortListModel::new(Some(filter_model.clone()), None::<gtk::CustomSorter>);
    SORT_MODEL.with(|s| *s.borrow_mut() = Some(sort_model.clone()));
    let selection = gtk::SingleSelection::new(Some(sort_model.clone()));
    let factory = gtk::SignalListItemFactory::new();

    let sf = state.clone();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let text_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text_box.set_hexpand(true);
        text_box.set_margin_start(12);
        text_box.set_margin_end(6);
        text_box.set_margin_top(8);
        text_box.set_margin_bottom(8);

        let name_lbl = gtk::Label::new(None);
        name_lbl.set_xalign(0.0);
        name_lbl.add_css_class("heading");
        text_box.append(&name_lbl);

        let summary_lbl = gtk::Label::new(None);
        summary_lbl.set_xalign(0.0);
        summary_lbl.add_css_class("caption");
        summary_lbl.add_css_class("dim-label");
        text_box.append(&summary_lbl);

        row.append(&text_box);

        let ver_lbl = gtk::Label::new(None);
        ver_lbl.set_xalign(1.0);
        ver_lbl.set_margin_end(6);
        ver_lbl.add_css_class("mono");
        row.append(&ver_lbl);

        let src_lbl = gtk::Label::new(None);
        src_lbl.set_margin_end(12);
        row.append(&src_lbl);

        let size_lbl = gtk::Label::new(None);
        size_lbl.set_xalign(1.0);
        size_lbl.set_margin_end(12);
        size_lbl.add_css_class("dim-label");
        size_lbl.set_width_request(80);
        row.append(&size_lbl);

        item.set_child(Some(&row));
    });

    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let row = item.child().and_downcast::<gtk::Box>().expect("row");

        let text_box = row.first_child().and_downcast::<gtk::Box>().expect("text_box");
        let name_lbl = text_box.first_child().and_downcast::<gtk::Label>().expect("name");
        let summary_lbl = name_lbl.next_sibling().and_downcast::<gtk::Label>().expect("summary");
        let ver_lbl = text_box.next_sibling().and_downcast::<gtk::Label>().expect("version");
        let src_lbl = ver_lbl.next_sibling().and_downcast::<gtk::Label>().expect("source");
        let size_lbl = src_lbl.next_sibling().and_downcast::<gtk::Label>().expect("size");

        let app = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|so| so.string())
            .and_then(|id| {
                let apps = sf.apps.lock().unwrap();
                apps.iter().find(|a| a.id == id.as_str()).cloned()
            });

        if let Some(ref a) = app {
            name_lbl.set_label(&a.display_name);
            summary_lbl.set_label(&a.summary);
            ver_lbl.set_label(&a.version);
            let (text, css) = source_badge(&a.source);
            src_lbl.set_label(text);
            src_lbl.set_css_classes(&[css]);
            size_lbl.set_label(&format_size(a.size_bytes.total_footprint));
        }
    });

    let list_view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list_view.add_css_class("boxed-list");

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list_view)
        .vexpand(true)
        .build();

    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scrolled));

    search_entry.connect_search_changed({
        let fm = filter_model.clone();
        move |entry| {
            let text = entry.text().to_lowercase();
            let filter = gtk::CustomFilter::new(move |obj| {
                if text.is_empty() {
                    return true;
                }
                obj.downcast_ref::<gtk::StringObject>()
                    .map(|s| s.string().to_lowercase().contains(&text))
                    .unwrap_or(true)
            });
            fm.set_filter(Some(&filter));
        }
    });

    {
        let state = state.clone();
        let sm = sort_model.clone();
        sort_dropdown.connect_notify_local(Some("selected"), move |dd, _| {
            let option = dd.selected();
            let sorter = build_sorter(option, &state);
            sm.set_sorter(Some(&sorter));
        });
    }
    let sel_state = state.clone();
    selection.connect_selection_changed(move |sel, _, _| {
        let selected = sel.selected_item();
        let app_id = selected.and_downcast::<gtk::StringObject>().map(|s| s.string());
        if let Some(id) = app_id {
            let apps = sel_state.apps.lock().unwrap();
            let app = apps.iter().find(|a| a.id == id.as_str()).cloned();
            if let Some(app) = app {
                show_detail_page(app, &sel_state);
            }
        }
    });

    SCAN_BTN.with(|b| *b.borrow_mut() = Some(scan_btn.clone()));

    scan_btn.connect_clicked({
        let state = state;
        move |_| {
            SCAN_BTN.with(|b| {
                if let Some(ref btn) = *b.borrow() {
                    btn.set_sensitive(false);
                    btn.set_label("Scanning…");
                }
            });
            SCAN_SPINNER.with(|s| {
                if let Some(ref sp) = *s.borrow() {
                    sp.set_visible(true);
                    sp.start();
                }
            });

            let state_for_task = state.clone();
            let main_ctx = glib::MainContext::default();

            std::thread::spawn(move || {
                let total = state_for_task.rt.block_on(async {
                    let cancel = tokio_util::sync::CancellationToken::new();
                    let result = state_for_task.discovery.scan_all(cancel).await;
                    *state_for_task.apps.lock().unwrap() =
                        state_for_task.discovery.get_cached_apps();
                    result.total
                });

                main_ctx.invoke(move || {
                    let ids: Vec<String> = {
                        let apps = state_for_task.apps.lock().unwrap();
                        apps.iter().map(|a| a.id.clone()).collect()
                    };

                    populate_store(&ids, total);
                    SORT_DROPDOWN.with(|dd| {
                        if let Some(ref dd) = *dd.borrow() {
                            let option = dd.selected();
                            SORT_MODEL.with(|sm| {
                                if let Some(ref sm) = *sm.borrow() {
                                    sm.set_sorter(Some(&build_sorter(option, &state_for_task)));
                                }
                            });
                        }
                    });
                    SCAN_SPINNER.with(|s| {
                        if let Some(ref sp) = *s.borrow() {
                            sp.stop();
                            sp.set_visible(false);
                        }
                    });
                });
            });
        }
    });

    adw::NavigationPage::builder()
        .title("Orbit")
        .child(&toolbar)
        .build()
}

fn build_sorter(option: u32, state: &Arc<SharedState>) -> gtk::CustomSorter {
    let state = state.clone();
    gtk::CustomSorter::new(move |a, b| {
        let aid = a
            .downcast_ref::<gtk::StringObject>()
            .map(|s| s.string());
        let bid = b
            .downcast_ref::<gtk::StringObject>()
            .map(|s| s.string());
        let apps = state.apps.lock().unwrap();
        let aa = aid.as_ref().and_then(|id| apps.iter().find(|app| app.id == id.as_str()));
        let bb = bid.as_ref().and_then(|id| apps.iter().find(|app| app.id == id.as_str()));
        match (aa, bb) {
        (Some(aa), Some(bb)) => match option {
            0 => aa.display_name.cmp(&bb.display_name).into(),
            1 => bb.display_name.cmp(&aa.display_name).into(),
            2 => bb.size_bytes.total_footprint.cmp(&aa.size_bytes.total_footprint).into(),
            _ => aa.source.cmp(&bb.source).into(),
        },
            _ => std::cmp::Ordering::Equal.into(),
        }
    })
}

fn populate_store(ids: &[String], total: usize) {
    APP_STORE.with(|s| {
        if let Some(ref store) = *s.borrow() {
            store.remove_all();
            for id in ids {
                store.append(&gtk::StringObject::new(id));
            }
        }
    });
    SCAN_BTN.with(|b| {
        if let Some(ref btn) = *b.borrow() {
            btn.set_sensitive(true);
            btn.set_label(&format!("Scan ({} apps)", total));
        }
    });
}

fn load_cached_apps(state: Arc<SharedState>) {
    let apps = state.discovery.get_cached_apps();
    let total = apps.len();
    let ids: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
    *state.apps.lock().unwrap() = apps;
    populate_store(&ids, total);

    SORT_DROPDOWN.with(|dd| {
        if let Some(ref dd) = *dd.borrow() {
            let option = dd.selected();
            SORT_MODEL.with(|sm| {
                if let Some(ref sm) = *sm.borrow() {
                    sm.set_sorter(Some(&build_sorter(option, &state)));
                }
            });
        }
    });
}

fn show_detail_page(app: AppFootprint, state: &Arc<SharedState>) {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let back_btn = gtk::Button::with_label("Back");
    header.pack_start(&back_btn);

    let stack = adw::ViewStack::new();
    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    header.set_title_widget(Some(&switcher));

    let uninstall_btn = gtk::Button::with_label("Uninstall");
    uninstall_btn.add_css_class("destructive-action");
    header.pack_end(&uninstall_btn);

    // ---- Info page ----
    let info_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    info_box.set_margin_start(24);
    info_box.set_margin_end(24);
    info_box.set_margin_top(24);
    info_box.set_margin_bottom(24);

    let name_lbl = gtk::Label::builder()
        .label(&app.display_name)
        .css_classes(["title-1"])
        .xalign(0.0)
        .wrap(true)
        .build();
    info_box.append(&name_lbl);

    let (st, sc) = source_badge(&app.source);
    let src_badge = gtk::Label::builder()
        .label(st)
        .css_classes([sc])
        .xalign(0.0)
        .build();
    info_box.append(&src_badge);

    if !app.summary.is_empty() {
        let sum_lbl = gtk::Label::builder()
            .label(&app.summary)
            .css_classes(["body"])
            .xalign(0.0)
            .wrap(true)
            .build();
        info_box.append(&sum_lbl);
    }

    if !app.description.is_empty() {
        let desc_lbl = gtk::Label::builder()
            .label(&app.description)
            .css_classes(["body"])
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        info_box.append(&desc_lbl);
    }

    let grid = gtk::Grid::new();
    grid.set_column_spacing(16);
    grid.set_row_spacing(8);
    grid.set_margin_top(16);

    let mut r = 0;
    add_row(&grid, r, "Version", &app.version);
    r += 1;
    add_row(&grid, r, "Architecture", &app.architecture);
    r += 1;
    add_row(&grid, r, "Source", &format!("{:?}", app.source));
    r += 1;
    add_row(&grid, r, "Package Size", &format_size(app.size_bytes.package_size));
    r += 1;
    add_row(&grid, r, "Total Footprint", &format_size(app.size_bytes.total_footprint));
    r += 1;
    if app.size_bytes.config_size > 0 {
        add_row(&grid, r, "  Config", &format_size(app.size_bytes.config_size));
        r += 1;
    }
    if app.size_bytes.data_size > 0 {
        add_row(&grid, r, "  Data", &format_size(app.size_bytes.data_size));
        r += 1;
    }
    if app.size_bytes.shared_size > 0 {
        add_row(&grid, r, "  Shared Libs", &format_size(app.size_bytes.shared_size));
        r += 1;
    }
    if app.size_bytes.cache_size > 0 {
        add_row(&grid, r, "  Cache", &format_size(app.size_bytes.cache_size));
    }

    info_box.append(&grid);

    let info_scroll = gtk::ScrolledWindow::builder()
        .child(&info_box)
        .vexpand(true)
        .build();
    stack.add_titled(&info_scroll, Some("info"), "Info");

    // ---- Dependencies page ----
    let deps_scroll = gtk::ScrolledWindow::new();
    let deps_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    deps_box.set_margin_start(12);
    deps_box.set_margin_end(12);
    deps_box.set_margin_top(12);
    deps_box.set_margin_bottom(12);

    let deps_spinner = gtk::Spinner::new();
    deps_spinner.start();
    deps_box.append(&deps_spinner);

    let deps_list_store = gio::ListStore::new::<gtk::StringObject>();
    let deps_factory = gtk::SignalListItemFactory::new();
    deps_factory.connect_setup(|_, item| {
        let lbl = gtk::Label::new(None);
        lbl.set_xalign(0.0);
        lbl.set_margin_start(8);
        lbl.set_margin_end(8);
        lbl.set_margin_top(4);
        lbl.set_margin_bottom(4);
        item.downcast_ref::<gtk::ListItem>()
            .expect("ListItem")
            .set_child(Some(&lbl));
    });
    deps_factory.connect_bind(|_, item| {
        let li = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        if let Some(obj) = li.item().and_downcast::<gtk::StringObject>() {
            if let Some(lbl) = li.child().and_downcast::<gtk::Label>() {
                lbl.set_label(&obj.string());
            }
        }
    });
    let deps_list_view = gtk::ListView::new(
        Some(gtk::SingleSelection::new(Some(deps_list_store.clone()))),
        Some(deps_factory),
    );
    deps_list_view.add_css_class("boxed-list");
    deps_box.append(&deps_list_view);

    deps_scroll.set_child(Some(&deps_box));
    stack.add_titled(&deps_scroll, Some("deps"), "Dependencies");

    // ---- Files page ----
    let files_scroll = gtk::ScrolledWindow::new();
    let files_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    files_box.set_margin_start(12);
    files_box.set_margin_end(12);
    files_box.set_margin_top(12);
    files_box.set_margin_bottom(12);

    let files_spinner = gtk::Spinner::new();
    files_spinner.start();
    files_box.append(&files_spinner);

    let files_list_store = gio::ListStore::new::<gtk::StringObject>();
    let files_factory = gtk::SignalListItemFactory::new();
    files_factory.connect_setup(|_, item| {
        let lbl = gtk::Label::new(None);
        lbl.set_xalign(0.0);
        lbl.set_margin_start(8);
        lbl.set_margin_end(8);
        lbl.set_margin_top(4);
        lbl.set_margin_bottom(4);
        lbl.add_css_class("mono");
        lbl.set_wrap(true);
        lbl.set_selectable(true);
        item.downcast_ref::<gtk::ListItem>()
            .expect("ListItem")
            .set_child(Some(&lbl));
    });
    files_factory.connect_bind(|_, item| {
        let li = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        if let Some(obj) = li.item().and_downcast::<gtk::StringObject>() {
            if let Some(lbl) = li.child().and_downcast::<gtk::Label>() {
                lbl.set_label(&obj.string());
            }
        }
    });
    let files_list_view = gtk::ListView::new(
        Some(gtk::SingleSelection::new(Some(files_list_store.clone()))),
        Some(files_factory),
    );
    files_list_view.add_css_class("boxed-list");
    files_box.append(&files_list_view);

    files_scroll.set_child(Some(&files_box));
    stack.add_titled(&files_scroll, Some("files"), "Files");

    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let page = adw::NavigationPage::builder()
        .title(&app.display_name)
        .child(&toolbar)
        .build();

    NAV_VIEW.with(|n| {
        if let Some(ref nav) = *n.borrow() {
            let nav_clone = nav.clone();
            back_btn.connect_clicked(move |_| {
                nav_clone.pop();
            });
            nav.push(&page);
        }
    });

    // ---- Load dependencies and files ----
    // Use block_on on the main thread — these queries are fast (<50ms each)
    let app_id_clone = app.id.clone();
    if let Some(adapter) = state.discovery.get_adapter(app.source) {
        match state.rt.block_on(adapter.list_dependencies(&app_id_clone)) {
            Ok(deps) => {
                deps_spinner.stop();
                deps_spinner.set_visible(false);
                for dep in &deps {
                    let label = if dep.version.is_empty() {
                        dep.name.clone()
                    } else {
                        format!("{} ({})", dep.name, dep.version)
                    };
                    deps_list_store.append(&gtk::StringObject::new(&label));
                }
            }
            Err(e) => {
                deps_spinner.stop();
                deps_spinner.set_visible(false);
                deps_list_store.append(&gtk::StringObject::new(&format!("(error: {})", e)));
            }
        }

        match state.rt.block_on(adapter.list_files(&app_id_clone)) {
            Ok(files) => {
                files_spinner.stop();
                files_spinner.set_visible(false);
                for f in &files {
                    files_list_store.append(&gtk::StringObject::new(f));
                }
            }
            Err(e) => {
                files_spinner.stop();
                files_spinner.set_visible(false);
                files_list_store.append(&gtk::StringObject::new(&format!("(error: {})", e)));
            }
        }
    }

    // ---- Uninstall dialog ----
    let state_for_dialog = state.clone();
    let app_clone = app.clone();
    uninstall_btn.connect_clicked(move |_| {
        show_uninstall_dialog(&app_clone, &state_for_dialog);
    });
}

fn show_uninstall_dialog(app: &AppFootprint, state: &Arc<SharedState>) {
    let parent = APP_WINDOW.with(|w| w.borrow().clone());
    let dialog = adw::MessageDialog::new(
        parent.as_ref(),
        Some(&format!("Uninstall {}", app.display_name)),
        Some("Loading uninstall plan…"),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("uninstall", "Uninstall");
    dialog.set_response_appearance("uninstall", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    // Load uninstall plan synchronously (brief block_on)
    let plan = state.rt.block_on(async {
        match state.discovery.get_adapter(app.source) {
            Some(a) => a.plan_uninstall(
                &app.id,
                crate::models::UninstallOptions::default(),
            ).await,
            None => Err(AdapterError::NotAvailable("no adapter".into())),
        }
    });

    let plan = match plan {
        Ok(p) => {
            let stages: Vec<_> = p
                .stages
                .iter()
                .map(|s| format!("• {}", s.description))
                .collect();
            dialog.set_body(&format!(
                "Uninstall plan ({} steps):\n{}\n\nTotal space to free: {}",
                p.stages.len(),
                stages.join("\n"),
                format_size(p.total_space_to_free),
            ));
            p
        }
        Err(e) => {
            dialog.set_body(&format!(
                "Could not generate uninstall plan:\n{}",
                e
            ));
            dialog.present();
            return;
        }
    };

    let plan_clone = plan.clone();
    let state_clone = state.clone();
    let display_name = app.display_name.clone();
    let source = app.source.clone();
    let app_id = app.id.clone();

    dialog.connect_response(None, move |d, response| {
        if response == "uninstall" {
            d.close();

            // Execute uninstall
            let result = state_clone.rt.block_on(async {
                if let Some(a) = state_clone.discovery.get_adapter(source) {
                    a.execute_uninstall(&plan_clone, CancellationToken::new()).await
                } else {
                    Err(AdapterError::NotAvailable("no adapter".into()))
                }
            });

            let (body_text, _success) = match result {
                Ok(r) if r.success => {
                    // Clean up database and UI
                    let db = state_clone.db.clone();
                    let _ = state_clone.rt.block_on(async {
                        let _ = db.delete_app(&app_id);
                    });
                    
                    // Remove from in-memory list
                    if let Ok(mut apps) = state_clone.apps.lock() {
                        apps.retain(|a| a.id != app_id);
                    }
                    
                    // Remove from UI list store
                    APP_STORE.with(|store| {
                        if let Some(ref s) = *store.borrow() {
                            let mut i = 0;
                            while i < s.n_items() {
                                if let Some(obj) = s.item(i).and_downcast::<gtk::StringObject>() {
                                    if obj.string().starts_with(&app_id) {
                                        s.remove(i);
                                        break;
                                    }
                                }
                                i += 1;
                            }
                        }
                    });
                    
                    // Pop detail view
                    NAV_VIEW.with(|n| {
                        if let Some(ref nav) = *n.borrow() {
                            nav.pop();
                        }
                    });
                    
                    (
                        format!(
                            "Successfully uninstalled {}\nSpace freed: {}",
                            display_name,
                            format_size(r.space_freed)
                        ),
                        true
                    )
                }
                Ok(r) => (
                    format!(
                        "Uninstall failed: {}\nSpace freed: {}",
                        r.error_message.unwrap_or_else(|| "Unknown error".into()),
                        format_size(r.space_freed)
                    ),
                    false
                ),
                Err(e) => (format!("Error: {}", e), false),
            };
            
            let result_dialog = adw::MessageDialog::new(
                parent.as_ref(),
                Some("Uninstall Result"),
                Some(&body_text),
            );
            result_dialog.add_response("ok", "OK");
            result_dialog.set_close_response("ok");
            result_dialog.connect_response(None, |rd, _| rd.close());
            result_dialog.present();
        } else {
            d.close();
        }
    });

    dialog.present();
}

fn add_row(grid: &gtk::Grid, row: i32, label: &str, value: &str) {
    let lbl = gtk::Label::builder()
        .label(label)
        .css_classes(["caption", "dim-label"])
        .xalign(0.0)
        .build();
    grid.attach(&lbl, 0, row, 1, 1);
    let val = gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .selectable(true)
        .build();
    grid.attach(&val, 1, row, 1, 1);
}

fn source_badge(source: &PackageSource) -> (&'static str, &'static str) {
    match source {
        PackageSource::Rpm => ("RPM", "badge-rpm"),
        PackageSource::Apt => ("DEB", "badge-deb"),
        PackageSource::Flatpak => ("Flatpak", "badge-flatpak"),
        PackageSource::Snap => ("Snap", "badge-snap"),
        PackageSource::Loose => ("AppImage", "badge-loose"),
        PackageSource::Unknown => ("?", "badge-unknown"),
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut s = bytes as f64;
    let mut i = 0;
    while s >= 1024.0 && i < UNITS.len() - 1 {
        s /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", bytes, UNITS[i])
    } else {
        format!("{:.1} {}", s, UNITS[i])
    }
}
