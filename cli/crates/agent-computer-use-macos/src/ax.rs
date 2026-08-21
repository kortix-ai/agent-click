use agent_computer_use_core::node::{AccessibilityNode, Point, Role, Size};
use agent_computer_use_core::selector::Selector;
use core_foundation::base::{CFType, TCFType};
use core_foundation::string::CFString;
use std::collections::VecDeque;
use std::ptr;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;

    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation::string::CFStringRef,
        value: *mut core_foundation::base::CFTypeRef,
    ) -> i32;

    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: core_foundation::string::CFStringRef,
        value: core_foundation::base::CFTypeRef,
    ) -> i32;

    fn AXIsProcessTrusted() -> bool;

    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;

    fn AXUIElementPerformAction(
        element: AXUIElementRef,
        action: core_foundation::string::CFStringRef,
    ) -> i32;

    fn AXUIElementCopyMultipleAttributeValues(
        element: AXUIElementRef,
        attributes: core_foundation::array::CFArrayRef,
        options: u32,
        values: *mut core_foundation::array::CFArrayRef,
    ) -> i32;
}

type AXUIElementRef = *mut std::ffi::c_void;

extern "C" {
    fn CFRetain(cf: core_foundation::base::CFTypeRef) -> core_foundation::base::CFTypeRef;
    fn CFRelease(cf: core_foundation::base::CFTypeRef);
}

const AX_ERROR_SUCCESS: i32 = 0;

pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn system_wide_element() -> AXUIElementRef {
    unsafe { AXUIElementCreateSystemWide() }
}

pub fn application_element(pid: i32) -> AXUIElementRef {
    unsafe { AXUIElementCreateApplication(pid) }
}

pub fn get_string_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    if element.is_null() {
        return None;
    }

    let cf_attr = CFString::new(attribute);
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let cf_type = unsafe { CFType::wrap_under_create_rule(value) };
    let type_id = cf_type.type_of();
    if type_id == core_foundation::string::CFString::type_id() {
        let cf_string: CFString =
            unsafe { CFString::wrap_under_get_rule(value as core_foundation::string::CFStringRef) };
        Some(cf_string.to_string())
    } else {
        None
    }
}

pub fn get_bool_attribute(element: AXUIElementRef, attribute: &str) -> Option<bool> {
    if element.is_null() {
        return None;
    }

    let cf_attr = CFString::new(attribute);
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let cf_type = unsafe { CFType::wrap_under_create_rule(value) };
    let type_id = cf_type.type_of();
    if type_id == core_foundation::boolean::CFBoolean::type_id() {
        let cf_bool = unsafe {
            core_foundation::boolean::CFBoolean::wrap_under_get_rule(
                value as core_foundation::boolean::CFBooleanRef,
            )
        };
        Some(cf_bool == core_foundation::boolean::CFBoolean::true_value())
    } else {
        None
    }
}

pub fn set_value(element: AXUIElementRef, value: &str) -> bool {
    if element.is_null() {
        return false;
    }
    let cf_attr = CFString::new("AXValue");
    let cf_value = CFString::new(value);
    let result = unsafe {
        AXUIElementSetAttributeValue(
            element,
            cf_attr.as_concrete_TypeRef(),
            cf_value.as_concrete_TypeRef() as core_foundation::base::CFTypeRef,
        )
    };
    result == AX_ERROR_SUCCESS
}

pub fn get_element_attribute(element: AXUIElementRef, attribute: &str) -> Option<AXUIElementRef> {
    if element.is_null() {
        return None;
    }
    let cf_attr = CFString::new(attribute);
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();
    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };
    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    Some(value as AXUIElementRef)
}

pub fn get_focused_element() -> Option<AccessibilityNode> {
    let system = system_wide_element();
    let focused = get_element_attribute(system, "AXFocusedUIElement")?;
    Some(element_to_node(focused, Some(0), 0))
}

pub fn set_focused(element: AXUIElementRef, focused: bool) -> bool {
    if element.is_null() {
        return false;
    }
    let cf_attr = CFString::new("AXFocused");
    let cf_value = if focused {
        core_foundation::boolean::CFBoolean::true_value()
    } else {
        core_foundation::boolean::CFBoolean::false_value()
    };
    let result = unsafe {
        AXUIElementSetAttributeValue(
            element,
            cf_attr.as_concrete_TypeRef(),
            cf_value.as_concrete_TypeRef() as core_foundation::base::CFTypeRef,
        )
    };
    result == AX_ERROR_SUCCESS
}

fn get_children_as_nodes(
    element: AXUIElementRef,
    max_depth: Option<u32>,
    current_depth: u32,
) -> Vec<AccessibilityNode> {
    if element.is_null() {
        return Vec::new();
    }

    let cf_attr = CFString::new("AXChildren");
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return Vec::new();
    }

    let cf_array = unsafe {
        core_foundation::array::CFArray::<CFType>::wrap_under_create_rule(
            value as core_foundation::array::CFArrayRef,
        )
    };

    let count = cf_array.len();
    let mut children = Vec::with_capacity(count as usize);

    for i in 0..count {
        if let Some(child) = cf_array.get(i) {
            let child_ref = child.as_CFTypeRef() as AXUIElementRef;
            if !child_ref.is_null() {
                children.push(element_to_node(child_ref, max_depth, current_depth));
            }
        }
    }

    children
}

pub fn get_position(element: AXUIElementRef) -> Option<Point> {
    if element.is_null() {
        return None;
    }

    let cf_attr = CFString::new("AXPosition");
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let mut point = core_graphics::geometry::CGPoint::new(0.0, 0.0);
    let success = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            AX_VALUE_TYPE_CGPOINT,
            &mut point as *mut _ as *mut std::ffi::c_void,
        )
    };

    if success {
        Some(Point {
            x: point.x,
            y: point.y,
        })
    } else {
        None
    }
}

pub fn get_size(element: AXUIElementRef) -> Option<Size> {
    if element.is_null() {
        return None;
    }

    let cf_attr = CFString::new("AXSize");
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let mut size = core_graphics::geometry::CGSize::new(0.0, 0.0);
    let success = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            AX_VALUE_TYPE_CGSIZE,
            &mut size as *mut _ as *mut std::ffi::c_void,
        )
    };

    if success {
        Some(Size {
            width: size.width,
            height: size.height,
        })
    } else {
        None
    }
}

pub fn get_pid(element: AXUIElementRef) -> Option<u32> {
    if element.is_null() {
        return None;
    }
    let mut pid: i32 = 0;
    let result = unsafe { AXUIElementGetPid(element, &mut pid) };
    if result == AX_ERROR_SUCCESS && pid > 0 {
        Some(pid as u32)
    } else {
        None
    }
}

const BATCH_ATTRS: &[&str] = &[
    "AXRole",
    "AXTitle",
    "AXDescription",
    "AXValue",
    "AXIdentifier",
    "AXPosition",
    "AXSize",
    "AXFocused",
    "AXEnabled",
    "AXChildren",
];

extern "C" {
    fn CFArrayCreate(
        allocator: *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        num_values: isize,
        callbacks: *const std::ffi::c_void,
    ) -> core_foundation::array::CFArrayRef;

    fn CFArrayGetCount(arr: core_foundation::array::CFArrayRef) -> isize;

    fn CFArrayGetValueAtIndex(
        arr: core_foundation::array::CFArrayRef,
        idx: isize,
    ) -> *const std::ffi::c_void;

    static kCFTypeArrayCallBacks: std::ffi::c_void;
}

fn batch_get_attributes(element: AXUIElementRef) -> Option<Vec<core_foundation::base::CFTypeRef>> {
    if element.is_null() {
        return None;
    }

    let cf_attrs: Vec<CFString> = BATCH_ATTRS.iter().map(|a| CFString::new(a)).collect();
    let cf_ptrs: Vec<*const std::ffi::c_void> = cf_attrs
        .iter()
        .map(|s| s.as_concrete_TypeRef() as *const std::ffi::c_void)
        .collect();

    let attr_array = unsafe {
        CFArrayCreate(
            ptr::null(),
            cf_ptrs.as_ptr(),
            cf_ptrs.len() as isize,
            &kCFTypeArrayCallBacks,
        )
    };

    if attr_array.is_null() {
        return None;
    }

    let mut values: core_foundation::array::CFArrayRef = ptr::null_mut();
    let result =
        unsafe { AXUIElementCopyMultipleAttributeValues(element, attr_array, 0, &mut values) };

    unsafe { CFRelease(attr_array as core_foundation::base::CFTypeRef) };

    if values.is_null() {
        return None;
    }

    let count = unsafe { CFArrayGetCount(values) };
    if count != BATCH_ATTRS.len() as isize {
        unsafe { CFRelease(values as core_foundation::base::CFTypeRef) };
        return None;
    }

    let _ = result;

    let mut out = Vec::with_capacity(BATCH_ATTRS.len());
    for i in 0..count {
        let val = unsafe { CFArrayGetValueAtIndex(values, i) };
        out.push(val as core_foundation::base::CFTypeRef);
    }

    Some(out)
}

fn cftype_to_string(value: core_foundation::base::CFTypeRef) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let type_id = unsafe { core_foundation::base::CFGetTypeID(value) };
    if type_id == core_foundation::string::CFString::type_id() {
        let cf_string: CFString =
            unsafe { CFString::wrap_under_get_rule(value as core_foundation::string::CFStringRef) };
        let s = cf_string.to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

fn cftype_to_value_string(value: core_foundation::base::CFTypeRef) -> Option<String> {
    if let Some(string) = cftype_to_string(value) {
        return Some(string);
    }

    if let Some(boolean) = cftype_to_bool(value) {
        return Some(boolean.to_string());
    }

    if value.is_null() {
        return None;
    }

    let type_id = unsafe { core_foundation::base::CFGetTypeID(value) };
    if type_id != core_foundation::number::CFNumber::type_id() {
        return None;
    }

    let number = unsafe {
        core_foundation::number::CFNumber::wrap_under_get_rule(
            value as core_foundation::number::CFNumberRef,
        )
    };
    let is_float =
        unsafe { core_foundation::number::CFNumberIsFloatType(number.as_concrete_TypeRef()) != 0 };

    if is_float {
        number.to_f64().map(|value| value.to_string())
    } else {
        number.to_i64().map(|value| value.to_string())
    }
}

fn get_value_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    if element.is_null() {
        return None;
    }

    let cf_attr = CFString::new(attribute);
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();
    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let _cf_type = unsafe { CFType::wrap_under_create_rule(value) };
    cftype_to_value_string(value)
}

fn value_from_selected_state(role: &Role, is_selected: Option<bool>) -> Option<String> {
    match role {
        Role::CheckBox | Role::Switch => is_selected.map(|selected| selected.to_string()),
        _ => None,
    }
}

fn selected_value_for_role(element: AXUIElementRef, role: &Role) -> Option<String> {
    match role {
        Role::CheckBox | Role::Switch => {
            value_from_selected_state(role, get_bool_attribute(element, "AXSelected"))
        }
        _ => None,
    }
}

fn cftype_to_bool(value: core_foundation::base::CFTypeRef) -> Option<bool> {
    if value.is_null() {
        return None;
    }
    let type_id = unsafe { core_foundation::base::CFGetTypeID(value) };
    if type_id == core_foundation::boolean::CFBoolean::type_id() {
        Some(value == core_foundation::boolean::CFBoolean::true_value().as_CFTypeRef())
    } else {
        None
    }
}

pub fn element_to_node(
    element: AXUIElementRef,
    max_depth: Option<u32>,
    current_depth: u32,
) -> AccessibilityNode {
    if let Some(attrs) = batch_get_attributes(element) {
        return element_to_node_batch(element, &attrs, max_depth, current_depth);
    }
    element_to_node_slow(element, max_depth, current_depth)
}

fn element_to_node_batch(
    element: AXUIElementRef,
    attrs: &[core_foundation::base::CFTypeRef],
    max_depth: Option<u32>,
    current_depth: u32,
) -> AccessibilityNode {
    let role_str = cftype_to_string(attrs[0]).unwrap_or_default();
    let role = map_role(&role_str);
    let title = cftype_to_string(attrs[1]);
    let description = cftype_to_string(attrs[2]);
    let name = title.or(description.clone());
    let value =
        cftype_to_value_string(attrs[3]).or_else(|| selected_value_for_role(element, &role));
    let id = cftype_to_string(attrs[4]);
    let position = extract_position(attrs[5]);
    let size = extract_size(attrs[6]);
    let focused = cftype_to_bool(attrs[7]);
    let enabled = cftype_to_bool(attrs[8]);
    let pid = get_pid(element);

    let children = if max_depth.is_none_or(|d| current_depth < d) {
        extract_children(attrs[9], max_depth, current_depth + 1)
    } else {
        Vec::new()
    };

    AccessibilityNode {
        role,
        name,
        value,
        description,
        id,
        position,
        size,
        focused,
        enabled,
        pid,
        children,
    }
}

fn element_to_node_slow(
    element: AXUIElementRef,
    max_depth: Option<u32>,
    current_depth: u32,
) -> AccessibilityNode {
    let role_str = get_string_attribute(element, "AXRole").unwrap_or_default();
    let role = map_role(&role_str);
    let name = get_string_attribute(element, "AXTitle")
        .or_else(|| get_string_attribute(element, "AXDescription"));
    let value =
        get_value_attribute(element, "AXValue").or_else(|| selected_value_for_role(element, &role));
    let description = get_string_attribute(element, "AXHelp");
    let id = get_string_attribute(element, "AXIdentifier");
    let position = get_position(element);
    let size = get_size(element);
    let focused = get_bool_attribute(element, "AXFocused");
    let enabled = get_bool_attribute(element, "AXEnabled");
    let pid = get_pid(element);

    let children = if max_depth.is_none_or(|d| current_depth < d) {
        get_children_as_nodes(element, max_depth, current_depth + 1)
    } else {
        Vec::new()
    };

    AccessibilityNode {
        role,
        name,
        value,
        description,
        id,
        position,
        size,
        focused,
        enabled,
        pid,
        children,
    }
}

fn extract_children(
    children_ref: core_foundation::base::CFTypeRef,
    max_depth: Option<u32>,
    current_depth: u32,
) -> Vec<AccessibilityNode> {
    if children_ref.is_null() {
        return Vec::new();
    }

    let type_id = unsafe { core_foundation::base::CFGetTypeID(children_ref) };
    if type_id != core_foundation::array::CFArray::<CFType>::type_id() {
        return Vec::new();
    }

    let cf_array = unsafe {
        core_foundation::array::CFArray::<CFType>::wrap_under_get_rule(
            children_ref as core_foundation::array::CFArrayRef,
        )
    };

    let count = cf_array.len();
    let mut children = Vec::with_capacity(count as usize);

    for i in 0..count {
        if let Some(child) = cf_array.get(i) {
            let child_ref = child.as_CFTypeRef() as AXUIElementRef;
            if !child_ref.is_null() {
                children.push(element_to_node(child_ref, max_depth, current_depth));
            }
        }
    }

    children
}

fn extract_position(value: core_foundation::base::CFTypeRef) -> Option<Point> {
    if value.is_null() {
        return None;
    }
    let mut point = core_graphics::geometry::CGPoint::new(0.0, 0.0);
    let ok = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            AX_VALUE_TYPE_CGPOINT,
            &mut point as *mut _ as *mut std::ffi::c_void,
        )
    };
    if ok {
        Some(Point {
            x: point.x,
            y: point.y,
        })
    } else {
        None
    }
}

fn extract_size(value: core_foundation::base::CFTypeRef) -> Option<Size> {
    if value.is_null() {
        return None;
    }
    let mut size = core_graphics::geometry::CGSize::new(0.0, 0.0);
    let ok = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            AX_VALUE_TYPE_CGSIZE,
            &mut size as *mut _ as *mut std::ffi::c_void,
        )
    };
    if ok {
        Some(Size {
            width: size.width,
            height: size.height,
        })
    } else {
        None
    }
}

fn map_role(ax_role: &str) -> Role {
    match ax_role {
        "AXApplication" => Role::Application,
        "AXWindow" => Role::Window,
        "AXDialog" | "AXAlert" => Role::Dialog,
        "AXSheet" => Role::Sheet,
        "AXPopover" => Role::Popover,
        "AXGroup" => Role::Group,
        "AXScrollArea" => Role::ScrollArea,
        "AXSplitGroup" => Role::SplitGroup,
        "AXTabGroup" => Role::TabGroup,
        "AXToolbar" => Role::Toolbar,
        "AXButton" => Role::Button,
        "AXCheckBox" => Role::CheckBox,
        "AXRadioButton" => Role::RadioButton,
        "AXTextField" => Role::TextField,
        "AXTextArea" => Role::TextArea,
        "AXSecureTextField" => Role::SecureTextField,
        "AXSlider" => Role::Slider,
        "AXStepper" | "AXIncrementor" => Role::Stepper,
        "AXSwitch" => Role::Switch,
        "AXComboBox" => Role::ComboBox,
        "AXPopUpButton" => Role::PopUpButton,
        "AXDisclosureTriangle" => Role::DisclosureTriangle,
        "AXColorWell" => Role::ColorWell,
        "AXDateField" => Role::DatePicker,
        "AXMenu" => Role::Menu,
        "AXMenuBar" => Role::MenuBar,
        "AXMenuItem" => Role::MenuItem,
        "AXMenuButton" => Role::MenuButton,
        "AXList" => Role::List,
        "AXRow" => Role::ListItem,
        "AXTable" => Role::Table,
        "AXTableRow" => Role::TableRow,
        "AXColumn" => Role::TableColumn,
        "AXOutline" => Role::Outline,
        "AXOutlineRow" => Role::OutlineRow,
        "AXBrowser" => Role::Tree,
        "AXTabButton" => Role::Tab,
        "AXTabPanel" => Role::TabPanel,
        "AXLink" => Role::Link,
        "AXNavigationBar" => Role::NavigationBar,
        "AXStaticText" => Role::StaticText,
        "AXImage" => Role::Image,
        "AXIcon" => Role::Icon,
        "AXProgressIndicator" => Role::ProgressIndicator,
        "AXBusyIndicator" => Role::BusyIndicator,
        "AXLevelIndicator" => Role::LevelIndicator,
        "AXWebArea" => Role::WebArea,
        "AXHeading" => Role::Heading,
        "AXParagraph" => Role::Paragraph,
        "AXBlockQuote" => Role::BlockQuote,
        "AXForm" => Role::Form,
        "AXArticle" => Role::Article,
        "AXBanner" => Role::Banner,
        "AXComplementary" => Role::Complementary,
        "AXContentInfo" => Role::ContentInfo,
        "AXMain" => Role::Main,
        "AXSearch" | "AXSearchField" => Role::Search,
        "AXSystemWide" => Role::SystemWide,
        "AXUnknown" | "" => Role::Unknown,
        other => Role::Other(other.to_string()),
    }
}

fn get_children_raw(element: AXUIElementRef) -> Vec<AXUIElementRef> {
    if element.is_null() {
        return Vec::new();
    }

    let cf_attr = CFString::new("AXChildren");
    let mut value: core_foundation::base::CFTypeRef = ptr::null_mut();

    let result = unsafe {
        AXUIElementCopyAttributeValue(element, cf_attr.as_concrete_TypeRef(), &mut value)
    };

    if result != AX_ERROR_SUCCESS || value.is_null() {
        return Vec::new();
    }

    let cf_array = unsafe {
        core_foundation::array::CFArray::<CFType>::wrap_under_create_rule(
            value as core_foundation::array::CFArrayRef,
        )
    };

    let count = cf_array.len();
    let mut children = Vec::with_capacity(count as usize);

    for i in 0..count {
        if let Some(child) = cf_array.get(i) {
            let child_ref = child.as_CFTypeRef() as AXUIElementRef;
            if !child_ref.is_null() {
                unsafe { CFRetain(child_ref as core_foundation::base::CFTypeRef) };
                children.push(child_ref);
            }
        }
    }

    children
}

const DEFAULT_SEARCH_DEPTH: u32 = 20;

const SKIP_ROLES: &[&str] = &["AXMenuBar", "AXMenu", "AXMenuItem"];

fn matches_selector_native(element: AXUIElementRef, selector: &Selector) -> bool {
    if let Some(ref role) = selector.role {
        let ax_role = get_string_attribute(element, "AXRole").unwrap_or_default();
        if map_role(&ax_role) != *role {
            return false;
        }
    }

    if let Some(ref id) = selector.id {
        let ax_id = get_string_attribute(element, "AXIdentifier");
        if ax_id.as_deref() != Some(id.as_str()) {
            return false;
        }
    }

    if let Some(ref sub) = selector.id_contains {
        let ax_id = get_string_attribute(element, "AXIdentifier").unwrap_or_default();
        if !ax_id.to_lowercase().contains(&sub.to_lowercase()) {
            return false;
        }
    }

    if let Some(ref name) = selector.name {
        let ax_name = get_string_attribute(element, "AXTitle")
            .or_else(|| get_string_attribute(element, "AXDescription"));
        if ax_name.as_deref() != Some(name.as_str()) {
            return false;
        }
    }

    if let Some(ref sub) = selector.name_contains {
        let ax_name = get_string_attribute(element, "AXTitle")
            .or_else(|| get_string_attribute(element, "AXDescription"))
            .unwrap_or_default();
        if !ax_name.to_lowercase().contains(&sub.to_lowercase()) {
            return false;
        }
    }

    true
}

fn should_skip_subtree(element: AXUIElementRef, selector: &Selector) -> bool {
    if selector.role.is_none() && selector.name.is_none() && selector.id.is_none() {
        return false;
    }
    let ax_role = get_string_attribute(element, "AXRole").unwrap_or_default();
    SKIP_ROLES.contains(&ax_role.as_str())
        && selector
            .role
            .as_ref()
            .is_none_or(|r| map_role(&ax_role) != *r)
}

pub fn find_first_native(root: AXUIElementRef, selector: &Selector) -> Option<AccessibilityNode> {
    let element = find_first_element(root, selector)?;
    let node = element_to_node(element, Some(0), 0);
    unsafe { CFRelease(element as core_foundation::base::CFTypeRef) };
    Some(node)
}

pub fn find_first_element(root: AXUIElementRef, selector: &Selector) -> Option<AXUIElementRef> {
    unsafe { CFRetain(root as core_foundation::base::CFTypeRef) };

    let max_depth = selector.max_depth.unwrap_or(DEFAULT_SEARCH_DEPTH);
    let mut queue = VecDeque::new();
    queue.push_back((root, 0u32));

    while let Some((element, depth)) = queue.pop_front() {
        if matches_selector_native(element, selector) {
            for (remaining, _) in &queue {
                unsafe { CFRelease(*remaining as core_foundation::base::CFTypeRef) };
            }
            return Some(element);
        }

        if depth < max_depth && !should_skip_subtree(element, selector) {
            for child in get_children_raw(element) {
                queue.push_back((child, depth + 1));
            }
        }

        unsafe { CFRelease(element as core_foundation::base::CFTypeRef) };
    }

    None
}

pub fn release_element(element: AXUIElementRef) {
    if !element.is_null() {
        unsafe { CFRelease(element as core_foundation::base::CFTypeRef) };
    }
}

pub fn perform_action(element: AXUIElementRef, action: &str) -> bool {
    let cf_action = CFString::new(action);
    let err = unsafe { AXUIElementPerformAction(element, cf_action.as_concrete_TypeRef()) };
    err == 0
}

pub fn raise_window(pid: i32) -> bool {
    let app = application_element(pid);
    let children = get_children_raw(app);
    for child in &children {
        let node = element_to_node(*child, Some(0), 0);
        if node.role == agent_computer_use_core::node::Role::Window {
            let result = perform_action(*child, "AXRaise");
            for c in &children {
                release_element(*c);
            }
            return result;
        }
    }
    for c in &children {
        release_element(*c);
    }
    false
}

pub fn press_element(root: AXUIElementRef, selector: &Selector) -> bool {
    if let Some(element) = find_first_element(root, selector) {
        let result = perform_action(element, "AXPress");
        release_element(element);
        result
    } else {
        false
    }
}

pub fn scroll_to_visible(root: AXUIElementRef, selector: &Selector) -> bool {
    if let Some(element) = find_first_element(root, selector) {
        let result = perform_action(element, "AXScrollToVisible");
        release_element(element);
        result
    } else {
        false
    }
}

pub fn find_all_native(root: AXUIElementRef, selector: &Selector) -> Vec<AccessibilityNode> {
    unsafe { CFRetain(root as core_foundation::base::CFTypeRef) };

    let max_depth = selector.max_depth.unwrap_or(DEFAULT_SEARCH_DEPTH);
    let mut queue = VecDeque::new();
    queue.push_back((root, 0u32));
    let mut results = Vec::new();

    while let Some((element, depth)) = queue.pop_front() {
        if matches_selector_native(element, selector) {
            results.push(element_to_node(element, Some(0), 0));
        }

        if depth < max_depth && !should_skip_subtree(element, selector) {
            for child in get_children_raw(element) {
                queue.push_back((child, depth + 1));
            }
        }

        unsafe { CFRelease(element as core_foundation::base::CFTypeRef) };
    }

    results
}

type AXValueRef = *mut std::ffi::c_void;
const AX_VALUE_TYPE_CGPOINT: u32 = 1;
const AX_VALUE_TYPE_CGSIZE: u32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXValueGetValue(
        value: AXValueRef,
        value_type: u32,
        value_ptr: *mut std::ffi::c_void,
    ) -> bool;

    fn AXValueCreate(value_type: u32, value_ptr: *const std::ffi::c_void) -> AXValueRef;
}

pub fn set_window_position(pid: i32, x: f64, y: f64) -> bool {
    let app = application_element(pid);
    let children = get_children_raw(app);
    for child in &children {
        let role = get_string_attribute(*child, "AXRole").unwrap_or_default();
        if role == "AXWindow" {
            let point = core_graphics::geometry::CGPoint::new(x, y);
            let ax_value = unsafe {
                AXValueCreate(
                    AX_VALUE_TYPE_CGPOINT,
                    &point as *const _ as *const std::ffi::c_void,
                )
            };
            if ax_value.is_null() {
                break;
            }
            let attr = CFString::new("AXPosition");
            let result = unsafe {
                AXUIElementSetAttributeValue(
                    *child,
                    attr.as_concrete_TypeRef(),
                    ax_value as core_foundation::base::CFTypeRef,
                )
            };
            unsafe { CFRelease(ax_value as core_foundation::base::CFTypeRef) };
            for c in &children {
                release_element(*c);
            }
            return result == AX_ERROR_SUCCESS;
        }
    }
    for c in &children {
        release_element(*c);
    }
    false
}

pub fn set_window_size(pid: i32, width: f64, height: f64) -> bool {
    let app = application_element(pid);
    let children = get_children_raw(app);
    for child in &children {
        let role = get_string_attribute(*child, "AXRole").unwrap_or_default();
        if role == "AXWindow" {
            let size = core_graphics::geometry::CGSize::new(width, height);
            let ax_value = unsafe {
                AXValueCreate(
                    AX_VALUE_TYPE_CGSIZE,
                    &size as *const _ as *const std::ffi::c_void,
                )
            };
            if ax_value.is_null() {
                break;
            }
            let attr = CFString::new("AXSize");
            let result = unsafe {
                AXUIElementSetAttributeValue(
                    *child,
                    attr.as_concrete_TypeRef(),
                    ax_value as core_foundation::base::CFTypeRef,
                )
            };
            unsafe { CFRelease(ax_value as core_foundation::base::CFTypeRef) };
            for c in &children {
                release_element(*c);
            }
            return result == AX_ERROR_SUCCESS;
        }
    }
    for c in &children {
        release_element(*c);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;

    #[test]
    fn converts_boolean_values_to_strings() {
        assert_eq!(
            cftype_to_value_string(
                core_foundation::boolean::CFBoolean::true_value().as_CFTypeRef()
            ),
            Some("true".to_string())
        );
        assert_eq!(
            cftype_to_value_string(
                core_foundation::boolean::CFBoolean::false_value().as_CFTypeRef()
            ),
            Some("false".to_string())
        );
    }

    #[test]
    fn converts_numeric_values_to_strings() {
        let integer = CFNumber::from(1_i64);
        let decimal = CFNumber::from(1.5_f64);

        assert_eq!(
            cftype_to_value_string(integer.as_CFTypeRef()),
            Some("1".to_string())
        );
        assert_eq!(
            cftype_to_value_string(decimal.as_CFTypeRef()),
            Some("1.5".to_string())
        );
    }

    #[test]
    fn converts_checkbox_and_switch_selected_states_to_values() {
        assert_eq!(
            value_from_selected_state(&Role::CheckBox, Some(true)),
            Some("true".to_string())
        );
        assert_eq!(
            value_from_selected_state(&Role::Switch, Some(false)),
            Some("false".to_string())
        );
        assert_eq!(value_from_selected_state(&Role::Button, Some(true)), None);
    }
}
