//! Top-bar icon and menu for GNOME and other Linux desktops.
//!
//! Implements the StatusNotifierItem and com.canonical.dbusmenu D-Bus
//! interfaces directly with Gio, like the Python app, so it needs no extra
//! packages. On Ubuntu the built-in "Ubuntu AppIndicators" extension shows it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gio;
use gtk::glib::{self, variant::ToVariant, Variant};

pub const ICON_NAME: &str = "io.github.aula_f87.Control-symbolic";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

const SNI_XML: &str = r#"<node><interface name="org.kde.StatusNotifierItem">
  <property name="Category" type="s" access="read"/>
  <property name="Id" type="s" access="read"/>
  <property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconName" type="s" access="read"/>
  <property name="IconThemePath" type="s" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
  <property name="Menu" type="o" access="read"/>
  <property name="XAyatanaLabel" type="s" access="read"/>
  <property name="XAyatanaLabelGuide" type="s" access="read"/>
  <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
  <signal name="NewTitle"/>
  <signal name="NewIcon"/>
  <signal name="NewToolTip"/>
  <signal name="NewStatus"><arg type="s"/></signal>
  <signal name="XAyatanaNewLabel"><arg type="s"/><arg type="s"/></signal>
</interface></node>"#;

const MENU_XML: &str = r#"<node><interface name="com.canonical.dbusmenu">
  <property name="Version" type="u" access="read"/>
  <property name="TextDirection" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconThemePath" type="as" access="read"/>
  <method name="GetLayout">
    <arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
    <arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/>
  </method>
  <method name="GetGroupProperties">
    <arg type="ai" direction="in"/><arg type="as" direction="in"/>
    <arg type="a(ia{sv})" direction="out"/>
  </method>
  <method name="GetProperty">
    <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
  </method>
  <method name="Event">
    <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/>
    <arg type="u" direction="in"/>
  </method>
  <method name="EventGroup">
    <arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/>
  </method>
  <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
  <method name="AboutToShowGroup">
    <arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/>
  </method>
  <signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})"/><arg type="a(ias)"/></signal>
  <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
  <signal name="ItemActivationRequested"><arg type="i"/><arg type="u"/></signal>
</interface></node>"#;

/// One menu entry. `toggle` is None, "checkmark" or "radio".
#[derive(Clone, Default)]
pub struct Item {
    pub label: String,
    pub action: Option<Rc<dyn Fn()>>,
    pub toggle: Option<&'static str>,
    pub checked: bool,
    pub enabled: bool,
    pub children: Vec<Item>,
    pub separator: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, action: impl Fn() + 'static) -> Item {
        Item { label: label.into(), action: Some(Rc::new(action)), enabled: true, ..Default::default() }
    }
    pub fn label(label: impl Into<String>) -> Item {
        Item { label: label.into(), enabled: false, ..Default::default() }
    }
    pub fn submenu(label: impl Into<String>, children: Vec<Item>) -> Item {
        Item { label: label.into(), enabled: true, children, ..Default::default() }
    }
    pub fn separator() -> Item {
        Item { separator: true, enabled: true, ..Default::default() }
    }
    pub fn toggle(mut self, kind: &'static str, checked: bool) -> Item {
        self.toggle = Some(kind);
        self.checked = checked;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }
}

struct Node {
    item: Item,
    children: Vec<i32>,
}

struct Inner {
    conn: gio::DBusConnection,
    title: String,
    icon_theme_path: String,
    build_menu: Box<dyn Fn() -> Vec<Item>>,
    on_activate: Box<dyn Fn()>,
    label: RefCell<String>,
    tooltip: RefCell<String>,
    revision: Cell<u32>,
    nodes: RefCell<HashMap<i32, Node>>,
    available: Cell<bool>,
    ids: RefCell<Vec<gio::RegistrationId>>,
    unwatch: RefCell<Option<Box<dyn FnOnce()>>>,
}

/// Publishes the icon and a menu built by `build_menu`. Call `refresh()`
/// whenever the state shown in the menu changes. `available()` is true while a
/// top-bar host is showing the icon.
#[derive(Clone)]
pub struct Tray(Rc<Inner>);

impl Tray {
    pub fn new(
        conn: &gio::DBusConnection,
        title: &str,
        icon_theme_path: &str,
        build_menu: impl Fn() -> Vec<Item> + 'static,
        on_activate: impl Fn() + 'static,
    ) -> Tray {
        let tray = Tray(Rc::new(Inner {
            conn: conn.clone(),
            title: title.into(),
            icon_theme_path: icon_theme_path.into(),
            build_menu: Box::new(build_menu),
            on_activate: Box::new(on_activate),
            label: RefCell::new(String::new()),
            tooltip: RefCell::new(title.into()),
            revision: Cell::new(1),
            nodes: RefCell::new(HashMap::new()),
            available: Cell::new(false),
            ids: RefCell::new(vec![]),
            unwatch: RefCell::new(None),
        }));
        tray.rebuild();

        let sni = gio::DBusNodeInfo::for_xml(SNI_XML).unwrap().lookup_interface("org.kde.StatusNotifierItem").unwrap();
        let menu = gio::DBusNodeInfo::for_xml(MENU_XML).unwrap().lookup_interface("com.canonical.dbusmenu").unwrap();
        let (a, b, c, d) = (tray.weak(), tray.weak(), tray.weak(), tray.weak());
        let item_id = conn
            .register_object(ITEM_PATH, &sni)
            .method_call(move |_, _, _, _, method, _params, invocation| {
                if let Some(t) = a.upgrade() {
                    if method == "Activate" || method == "SecondaryActivate" {
                        let t = Tray(t);
                        glib::idle_add_local_once(move || (t.0.on_activate)());
                    }
                }
                invocation.return_value(None);
            })
            .property(move |_, _, _, _, name| match b.upgrade() {
                Some(t) => Tray(t).item_property(name),
                None => "".to_variant(),
            })
            .build();
        let menu_id = conn
            .register_object(MENU_PATH, &menu)
            .method_call(move |_, _, _, _, method, params, invocation| match c.upgrade() {
                Some(t) => Tray(t).menu_call(method, &params, invocation),
                None => invocation.return_value(None),
            })
            .property(move |_, _, _, _, name| match d.upgrade() {
                Some(_) => match name {
                    "Version" => 3u32.to_variant(),
                    "TextDirection" => "ltr".to_variant(),
                    "Status" => "normal".to_variant(),
                    _ => Vec::<String>::new().to_variant(),
                },
                None => "".to_variant(),
            })
            .build();
        tray.0.ids.borrow_mut().extend(item_id.ok());
        tray.0.ids.borrow_mut().extend(menu_id.ok());

        let (e, f) = (tray.weak(), tray.weak());
        let watch = gio::bus_watch_name_on_connection(
            conn,
            WATCHER,
            gio::BusNameWatcherFlags::NONE,
            move |conn, _, _| {
                if let Some(t) = e.upgrade() {
                    let unique = conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
                    conn.call(
                        Some(WATCHER),
                        "/StatusNotifierWatcher",
                        WATCHER,
                        "RegisterStatusNotifierItem",
                        Some(&(unique,).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        -1,
                        gio::Cancellable::NONE,
                        move |result| t.available.set(result.is_ok()),
                    );
                }
            },
            move |_, _| {
                if let Some(t) = f.upgrade() {
                    t.available.set(false);
                }
            },
        );
        *tray.0.unwatch.borrow_mut() = Some(Box::new(move || gio::bus_unwatch_name(watch)));
        tray
    }

    fn weak(&self) -> std::rc::Weak<Inner> {
        Rc::downgrade(&self.0)
    }

    pub fn available(&self) -> bool {
        self.0.available.get()
    }

    /// Rebuild the menu; optionally update the text next to the icon.
    pub fn refresh(&self, label: Option<&str>, tooltip: Option<&str>) {
        self.rebuild();
        let rev = self.0.revision.get() + 1;
        self.0.revision.set(rev);
        self.emit(MENU_PATH, "com.canonical.dbusmenu", "LayoutUpdated", Some((rev, 0i32).to_variant()));
        if let Some(label) = label {
            if *self.0.label.borrow() != label {
                *self.0.label.borrow_mut() = label.into();
                self.emit(ITEM_PATH, "org.kde.StatusNotifierItem", "XAyatanaNewLabel", Some((label, "100%").to_variant()));
            }
        }
        if let Some(tooltip) = tooltip {
            if *self.0.tooltip.borrow() != tooltip {
                *self.0.tooltip.borrow_mut() = tooltip.into();
                self.emit(ITEM_PATH, "org.kde.StatusNotifierItem", "NewToolTip", None);
            }
        }
    }

    pub fn close(&self) {
        if let Some(unwatch) = self.0.unwatch.borrow_mut().take() {
            unwatch();
        }
        for id in self.0.ids.borrow_mut().drain(..) {
            let _ = self.0.conn.unregister_object(id);
        }
        self.0.available.set(false);
    }

    fn emit(&self, path: &str, iface: &str, signal: &str, params: Option<Variant>) {
        let _ = self.0.conn.emit_signal(None, path, iface, signal, params.as_ref());
    }

    fn item_property(&self, name: &str) -> Variant {
        let i = &self.0;
        match name {
            "Category" => "Hardware".to_variant(),
            "Id" => "aula-f87-control".to_variant(),
            "Title" => i.title.to_variant(),
            "Status" => "Active".to_variant(),
            "IconName" => ICON_NAME.to_variant(),
            "IconThemePath" => i.icon_theme_path.to_variant(),
            "ToolTip" => (ICON_NAME, Vec::<(i32, i32, Vec<u8>)>::new(), i.title.as_str(), i.tooltip.borrow().as_str())
                .to_variant(),
            "ItemIsMenu" => false.to_variant(),
            "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH.to_string()).unwrap().to_variant(),
            "XAyatanaLabel" => i.label.borrow().to_variant(),
            "XAyatanaLabelGuide" => "100%".to_variant(),
            _ => "".to_variant(),
        }
    }

    /// Number the items depth-first; ids stay stable while the shape is the same.
    fn rebuild(&self) {
        let mut nodes = HashMap::new();
        nodes.insert(0, Node { item: Item::default(), children: vec![] });
        let mut counter = 0;
        fn add(nodes: &mut HashMap<i32, Node>, counter: &mut i32, parent: i32, items: Vec<Item>) {
            for item in items {
                *counter += 1;
                let id = *counter;
                let children = item.children.clone();
                nodes.insert(id, Node { item, children: vec![] });
                nodes.get_mut(&parent).unwrap().children.push(id);
                if !children.is_empty() {
                    add(nodes, counter, id, children);
                }
            }
        }
        add(&mut nodes, &mut counter, 0, (self.0.build_menu)());
        *self.0.nodes.borrow_mut() = nodes;
    }

    fn props(&self, id: i32) -> HashMap<String, Variant> {
        let nodes = self.0.nodes.borrow();
        let mut p = HashMap::new();
        let Some(node) = nodes.get(&id) else { return p };
        if id == 0 {
            p.insert("children-display".into(), "submenu".to_variant());
            return p;
        }
        let item = &node.item;
        if item.separator {
            p.insert("type".into(), "separator".to_variant());
            return p;
        }
        p.insert("label".into(), item.label.replace('_', "__").to_variant());
        p.insert("enabled".into(), item.enabled.to_variant());
        if let Some(kind) = item.toggle {
            p.insert("toggle-type".into(), kind.to_variant());
            p.insert("toggle-state".into(), (item.checked as i32).to_variant());
        }
        if !item.children.is_empty() {
            p.insert("children-display".into(), "submenu".to_variant());
        }
        p
    }

    fn layout(&self, id: i32, depth: i32) -> Variant {
        let children: Vec<Variant> = if depth != 0 {
            let ids = self.0.nodes.borrow().get(&id).map(|n| n.children.clone()).unwrap_or_default();
            ids.into_iter().map(|c| self.layout(c, depth - 1)).collect()
        } else {
            vec![]
        };
        (id, self.props(id), children).to_variant()
    }

    fn click(&self, id: i32) {
        let action = self.0.nodes.borrow().get(&id).and_then(|n| {
            (n.item.enabled).then(|| n.item.action.clone()).flatten()
        });
        if let Some(action) = action {
            glib::idle_add_local_once(move || action());
        }
    }

    fn menu_call(&self, method: &str, params: &Variant, invocation: gio::DBusMethodInvocation) {
        match method {
            "GetLayout" => {
                let (mut parent, depth, _names) = params.get::<(i32, i32, Vec<String>)>().unwrap_or((0, -1, vec![]));
                if !self.0.nodes.borrow().contains_key(&parent) {
                    parent = 0;
                }
                let layout = self.layout(parent, depth);
                let reply = Variant::tuple_from_iter([self.0.revision.get().to_variant(), layout]);
                invocation.return_value(Some(&reply));
            }
            "GetGroupProperties" => {
                let (ids, _names) = params.get::<(Vec<i32>, Vec<String>)>().unwrap_or_default();
                let ids = if ids.is_empty() {
                    let mut all: Vec<i32> = self.0.nodes.borrow().keys().cloned().collect();
                    all.sort();
                    all
                } else {
                    ids
                };
                let known: Vec<(i32, HashMap<String, Variant>)> = ids
                    .into_iter()
                    .filter(|i| self.0.nodes.borrow().contains_key(i))
                    .map(|i| (i, self.props(i)))
                    .collect();
                invocation.return_value(Some(&(known,).to_variant()));
            }
            "GetProperty" => {
                let (id, name) = params.get::<(i32, String)>().unwrap_or((0, String::new()));
                let value = self.props(id).remove(&name).unwrap_or_else(|| "".to_variant());
                invocation.return_value(Some(&Variant::tuple_from_iter([Variant::from_variant(&value)])));
            }
            "Event" => {
                if let Some((id, event, _data, _time)) = params.get::<(i32, String, Variant, u32)>() {
                    if event == "clicked" {
                        self.click(id);
                    }
                }
                invocation.return_value(None);
            }
            "EventGroup" => {
                if let Some((events,)) = params.get::<(Vec<(i32, String, Variant, u32)>,)>() {
                    for (id, event, _, _) in events {
                        if event == "clicked" {
                            self.click(id);
                        }
                    }
                }
                invocation.return_value(Some(&(Vec::<i32>::new(),).to_variant()));
            }
            "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
            "AboutToShowGroup" => invocation.return_value(Some(&(Vec::<i32>::new(), Vec::<i32>::new()).to_variant())),
            _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        }
    }
}
