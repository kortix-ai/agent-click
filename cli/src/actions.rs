use agent_computer_use_core::action::{Action, ActionResult, MouseButton, ScrollDirection};
use agent_computer_use_core::selector::{Selector, SelectorChain};
use agent_computer_use_core::Platform;
use std::time::Duration;

use crate::selector_dsl;
use crate::snapshot;
use crate::wait;

pub const POLL_INTERVAL: Duration = Duration::from_millis(200);
const VERIFY_DELAY: Duration = Duration::from_millis(30);
const TYPE_STEP_DELAY: Duration = Duration::from_millis(100);
const CDP_ID_PREFIX: &str = "__cdp:";

fn is_cdp_element(selector: &Selector) -> bool {
    selector
        .id
        .as_ref()
        .is_some_and(|id| id.starts_with(CDP_ID_PREFIX))
}

fn is_cdp_node(node: &agent_computer_use_core::AccessibilityNode) -> bool {
    node.id
        .as_ref()
        .is_some_and(|id| id.starts_with(CDP_ID_PREFIX))
}

pub fn parse_selector(dsl: &str) -> Result<SelectorChain, agent_computer_use_core::Error> {
    if dsl.starts_with('@') {
        return snapshot::resolve_ref(dsl);
    }
    selector_dsl::parse(dsl).map_err(|e| agent_computer_use_core::Error::PlatformError {
        message: format!("invalid selector: {e}"),
    })
}

pub fn parse_selector_with_app(
    dsl: &str,
    app: Option<&str>,
) -> Result<SelectorChain, agent_computer_use_core::Error> {
    let mut chain = parse_selector(dsl)?;
    if let Some(a) = app {
        if chain.selectors[0].app.is_none() {
            chain.selectors[0].app = Some(a.to_string());
        }
    }
    Ok(chain)
}

pub async fn find_element(
    platform: &dyn Platform,
    chain: &SelectorChain,
    timeout: Duration,
) -> agent_computer_use_core::Result<agent_computer_use_core::AccessibilityNode> {
    wait::poll_for_one_element(platform, chain, timeout, POLL_INTERVAL).await
}

fn selector_from_node(
    node: &agent_computer_use_core::AccessibilityNode,
    chain: &SelectorChain,
) -> Selector {
    Selector {
        app: chain.first().app.clone(),
        role: Some(node.role.clone()),
        name: node.name.clone(),
        name_contains: None,
        id: node.id.clone(),
        id_contains: None,
        max_depth: None,
        path: chain.first().path.clone(),
        index: None,
        css: None,
    }
}

pub async fn click(
    platform: &dyn Platform,
    chain: &SelectorChain,
    button: MouseButton,
    count: u32,
    timeout: Duration,
) -> agent_computer_use_core::Result<ActionResult> {
    // CDP elements — always JS click, no coordinates
    if is_cdp_element(chain.first()) && button == MouseButton::Left {
        let sel = chain.first().clone();
        let name = sel.name.as_deref().unwrap_or("element").to_string();
        match platform.press(&sel).await {
            Ok(true) => {
                return Ok(ActionResult {
                    success: true,
                    message: Some(format!("pressed {:?} via CDP", name)),
                    path: None,
                    data: None,
                });
            }
            Ok(false) => {
                return Err(agent_computer_use_core::Error::PlatformError {
                    message: format!("CDP press failed for {:?}", name),
                })
            }
            Err(e) => return Err(e),
        }
    }

    let node = find_element(platform, chain, timeout).await?;
    tracing::debug!(
        "click target: {:?} at {:?} pid={:?}",
        node.name,
        node.position,
        node.pid
    );

    // CDP node found via search — JS click
    if is_cdp_node(&node) && button == MouseButton::Left {
        let press_sel = selector_from_node(&node, chain);
        if let Ok(true) = platform.press(&press_sel).await {
            return Ok(ActionResult {
                success: true,
                message: Some(format!(
                    "pressed {:?} via CDP",
                    node.name.as_deref().unwrap_or("element")
                )),
                path: None,
                data: None,
            });
        }
    }

    let press_sel = selector_from_node(&node, chain);
    let name = node.name.as_deref().unwrap_or("element");

    if button == MouseButton::Left && count == 1 {
        match platform.press(&press_sel).await {
            Ok(true) => {
                let pos = node
                    .center()
                    .map(|c| format!(" at ({}, {})", c.x, c.y))
                    .unwrap_or_default();
                return Ok(ActionResult {
                    success: true,
                    message: Some(format!("pressed {:?}{}", name, pos)),
                    path: None,
                    data: None,
                });
            }
            Ok(false) => {
                tracing::debug!(
                    "AXPress unsupported for {:?}, falling back to coordinates",
                    name
                );
            }
            Err(e) => {
                tracing::debug!(
                    "AXPress failed for {:?}: {e}, falling back to coordinates",
                    name
                );
            }
        }
    }

    let center = node
        .center()
        .ok_or_else(|| agent_computer_use_core::Error::PlatformError {
            message: "element has no position/size and AXPress is unsupported".into(),
        })?;

    agent_computer_use_core::element::check_visible(&node)?;

    if count > 1 || button != MouseButton::Left {
        tracing::debug!(
      "using coordinate click (count={count}, button={button:?}) — requires window activation"
    );
    } else {
        tracing::debug!(
            "AXPress failed, falling back to coordinate click — requires window activation"
        );
    }

    if let Some(ref app_name) = chain.first().app {
        platform.activate(app_name).await?;
    }
    let selector = chain.first().app.as_ref().map(|_| Selector {
        app: chain.first().app.clone(),
        ..Selector::default()
    });

    platform
        .perform(&Action::Click {
            selector,
            coordinates: Some(center),
            button,
            count,
        })
        .await
}

pub async fn type_into(
    platform: &dyn Platform,
    chain: &SelectorChain,
    text: &str,
    submit: bool,
    timeout: Duration,
) -> agent_computer_use_core::Result<ActionResult> {
    if is_cdp_element(chain.first()) {
        let sel = chain.first().clone();
        match platform.set_value(&sel, text).await {
            Ok(true) => {
                if submit {
                    tokio::time::sleep(TYPE_STEP_DELAY).await;
                    platform
                        .perform(&Action::KeyPress {
                            key: "return".into(),
                            app: chain.first().app.clone(),
                        })
                        .await?;
                }
                let msg = if submit {
                    format!("typed {} characters and submitted via CDP", text.len())
                } else {
                    format!("typed {} characters via CDP", text.len())
                };
                return Ok(ActionResult {
                    success: true,
                    message: Some(msg),
                    path: None,
                    data: None,
                });
            }
            Ok(false) => tracing::debug!("CDP set_value returned false, falling back"),
            Err(e) => tracing::debug!("CDP set_value failed: {e}, falling back"),
        }
    }

    let node = find_element(platform, chain, timeout).await?;
    tracing::debug!("type target: {:?} at {:?}", node.name, node.position);

    let set_sel = selector_from_node(&node, chain);

    match platform.set_value(&set_sel, text).await {
        Ok(true) => {
            tracing::debug!("AXValue returned success, verifying...");
            tokio::time::sleep(VERIFY_DELAY).await;

            if is_cdp_node(&node) {
                if submit {
                    tokio::time::sleep(TYPE_STEP_DELAY).await;
                    platform
                        .perform(&Action::KeyPress {
                            key: "return".into(),
                            app: chain.first().app.clone(),
                        })
                        .await?;
                }
                let msg = if submit {
                    format!("typed {} characters and submitted via CDP", text.len())
                } else {
                    format!("typed {} characters via CDP", text.len())
                };
                return Ok(ActionResult {
                    success: true,
                    message: Some(msg),
                    path: None,
                    data: None,
                });
            }

            let value_set = wait::find_by_chain(platform, chain)
                .await
                .ok()
                .and_then(|results| results.into_iter().next())
                .map(|new_node| {
                    new_node
                        .value
                        .as_ref()
                        .map(|v| v.contains(text))
                        .unwrap_or(false)
                })
                .unwrap_or(false);

            if value_set {
                tracing::debug!("AXValue verified");

                if submit {
                    tokio::time::sleep(TYPE_STEP_DELAY).await;
                    platform
                        .perform(&Action::KeyPress {
                            key: "return".into(),
                            app: chain.first().app.clone(),
                        })
                        .await?;
                }

                let msg = if submit {
                    format!("typed {} characters and submitted", text.len())
                } else {
                    format!("typed {} characters", text.len())
                };
                return Ok(ActionResult {
                    success: true,
                    message: Some(msg),
                    path: None,
                    data: None,
                });
            }

            tracing::debug!("AXValue didn't stick, falling back to keyboard simulation");
        }
        Ok(false) => tracing::debug!("AXValue unsupported, falling back to keyboard simulation"),
        Err(e) => tracing::debug!("AXValue failed: {e}, falling back to keyboard simulation"),
    }

    tracing::debug!("using keyboard simulation — requires window activation");

    if let Some(ref app_name) = chain.first().app {
        platform.activate(app_name).await?;
    }

    if let Some(center) = node.center() {
        platform
            .perform(&Action::Click {
                selector: None,
                coordinates: Some(center),
                button: MouseButton::Left,
                count: 1,
            })
            .await?;
        tokio::time::sleep(TYPE_STEP_DELAY).await;
    }

    platform
        .perform(&Action::KeyPress {
            key: "cmd+a".into(),
            app: None,
        })
        .await?;
    tokio::time::sleep(Duration::from_millis(50)).await;

    platform
        .perform(&Action::KeyPress {
            key: "backspace".into(),
            app: None,
        })
        .await?;
    tokio::time::sleep(TYPE_STEP_DELAY).await;

    platform
        .perform(&Action::Type {
            text: text.to_string(),
            selector: None,
            submit,
        })
        .await
}

pub fn parse_direction(direction: &str) -> agent_computer_use_core::Result<ScrollDirection> {
    ScrollDirection::parse(direction).ok_or_else(|| agent_computer_use_core::Error::PlatformError {
        message: format!(
            "invalid scroll direction: '{}' (use up/down/left/right)",
            direction
        ),
    })
}
