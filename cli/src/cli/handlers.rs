use agent_computer_use_core::action::{Action, ActionResult, MouseButton};
use agent_computer_use_core::node::Point;
use agent_computer_use_core::selector::{Selector, SelectorChain};
use agent_computer_use_core::Platform;
use std::time::Duration;

use crate::actions;
use crate::batch;
use crate::cli::args::{Cli, Command};
use crate::cli::output::{ExpectOutcome, ExpectResult, Output, RunError};
use crate::observe;
use crate::snapshot;
use crate::wait;
use crate::workflow;

pub async fn run(
    command: Command,
    platform: &dyn Platform,
    output: &Output,
    timeout: Duration,
) -> Result<(), RunError> {
    match command {
        Command::Tree { app, pid, depth } => {
            let app = app_target(app, pid);
            let tree = platform.tree(app.as_deref(), depth).await?;
            output.print(&tree);
        }

        Command::Find {
            selector,
            app,
            pid,
            depth,
        } => {
            let app = app_target(app, pid);
            let mut chain = actions::parse_selector_with_app(&selector, app.as_deref())?;
            if let Some(d) = depth {
                chain.selectors[0].max_depth = Some(d);
            }
            let results = wait::find_by_chain(platform, &chain).await?;
            output.print(&results);
        }

        Command::GetValue { selector, app, pid } => {
            let app = app_target(app, pid);
            let chain = actions::parse_selector_with_app(&selector, app.as_deref())?;
            let node = actions::find_element(platform, &chain, timeout).await?;
            output.print(&node);
        }

        Command::Click {
            selector,
            app,
            pid,
            x,
            y,
            button,
            count,
            expect,
        } => {
            let app = app_target(app, pid);
            let btn = parse_mouse_button(button.as_deref());
            let cnt = count.unwrap_or(1);

            let result = if let Some(sel) = selector {
                let chain = actions::parse_selector_with_app(&sel, app.as_deref())?;
                actions::click(platform, &chain, btn, cnt, timeout).await?
            } else {
                let coordinates = match (x, y) {
                    (Some(x), Some(y)) => Some(Point { x, y }),
                    _ => None,
                };
                if let Some(ref app_name) = app {
                    platform.activate(app_name).await?;
                }
                let selector = app.as_ref().map(|app| Selector::new().with_app(app));
                platform
                    .perform(&Action::Click {
                        selector,
                        coordinates,
                        button: btn,
                        count: cnt,
                    })
                    .await?
            };

            output_or_expect(platform, result, expect, timeout, output).await?;
        }

        Command::Type {
            text,
            selector,
            app,
            pid,
            submit,
            append,
            expect,
        } => {
            let app = app_target(app, pid);
            let result = if let Some(sel) = selector {
                let chain = actions::parse_selector_with_app(&sel, app.as_deref())?;
                actions::type_into(platform, &chain, &text, submit, timeout).await?
            } else {
                if let Some(ref app_name) = app {
                    platform.activate(app_name).await?;
                }
                if !append {
                    platform
                        .perform(&Action::KeyPress {
                            key: "cmd+a".into(),
                            app: None,
                        })
                        .await?;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                platform
                    .perform(&Action::Type {
                        text,
                        selector: None,
                        submit,
                    })
                    .await?
            };

            output_or_expect(platform, result, expect, timeout, output).await?;
        }

        Command::Key {
            key,
            app,
            pid,
            expect,
        } => {
            let app = app_target(app, pid);
            let result = platform.perform(&Action::KeyPress { key, app }).await?;
            output_or_expect(platform, result, expect, timeout, output).await?;
        }

        Command::Drag {
            from,
            to,
            app,
            pid,
            from_x,
            from_y,
            to_x,
            to_y,
        } => {
            let app = app_target(app, pid);
            let from_point = if let Some(sel) = from {
                let chain = actions::parse_selector_with_app(&sel, app.as_deref())?;
                let node = actions::find_element(platform, &chain, timeout).await?;
                node.center()
                    .ok_or_else(|| agent_computer_use_core::Error::PlatformError {
                        message: "drag source has no position".into(),
                    })?
            } else {
                match (from_x, from_y) {
                    (Some(x), Some(y)) => Point { x, y },
                    _ => {
                        return Err(agent_computer_use_core::Error::PlatformError {
                            message: "drag requires either a selector or --from-x/--from-y".into(),
                        }
                        .into())
                    }
                }
            };

            let to_point = if let Some(sel) = to {
                let chain = actions::parse_selector_with_app(&sel, app.as_deref())?;
                let node = actions::find_element(platform, &chain, timeout).await?;
                node.center()
                    .ok_or_else(|| agent_computer_use_core::Error::PlatformError {
                        message: "drag target has no position".into(),
                    })?
            } else {
                match (to_x, to_y) {
                    (Some(x), Some(y)) => Point { x, y },
                    _ => {
                        return Err(agent_computer_use_core::Error::PlatformError {
                            message: "drag requires either a selector or --to-x/--to-y".into(),
                        }
                        .into())
                    }
                }
            };

            if let Some(ref app_name) = app {
                platform.activate(app_name).await?;
            }

            let result = platform
                .perform(&Action::Drag {
                    from: from_point,
                    to: to_point,
                })
                .await?;
            output.print(&result);
        }

        Command::ScrollTo { selector, app, pid } => {
            let app = app_target(app, pid);
            let chain = actions::parse_selector_with_app(&selector, app.as_deref())?;
            let node = actions::find_element(platform, &chain, timeout).await?;
            let sel = agent_computer_use_core::Selector {
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
            };
            let scrolled = platform.scroll_to_visible(&sel).await?;
            output.print(&ActionResult {
                success: scrolled,
                message: Some(if scrolled {
                    format!(
                        "scrolled {:?} into view",
                        node.name.as_deref().unwrap_or("element")
                    )
                } else {
                    "element does not support scroll-to-visible".into()
                }),
                path: None,
                data: None,
            });
        }

        Command::Scroll {
            direction,
            amount,
            app,
            pid,
            at_selector,
            expect,
        } => {
            let app = app_target(app, pid);
            let dir = actions::parse_direction(&direction)?;

            if let Some(at_dsl) = at_selector {
                let chain = actions::parse_selector_with_app(&at_dsl, app.as_deref())?;
                let node = actions::find_element(platform, &chain, timeout).await?;
                let center =
                    node.center()
                        .ok_or_else(|| agent_computer_use_core::Error::PlatformError {
                            message: "element has no position/size".into(),
                        })?;
                platform
                    .perform(&Action::MoveMouse {
                        selector: None,
                        coordinates: Some(center),
                    })
                    .await?;
            }

            let result = platform
                .perform(&Action::Scroll {
                    direction: dir,
                    amount: amount.unwrap_or(3),
                    selector: None,
                    app,
                })
                .await?;
            output_or_expect(platform, result, expect, timeout, output).await?;
        }

        Command::WaitFor { selector, interval } => {
            let chain = actions::parse_selector(&selector)?;
            let interval = Duration::from_millis(interval);
            let element = wait::poll_for_element(platform, &chain, timeout, interval).await?;
            output.print(&element);
        }

        Command::EnsureText { selector, text } => {
            let chain = actions::parse_selector(&selector)?;
            let node = actions::find_element(platform, &chain, timeout).await?;

            if let Some(ref current_value) = node.value {
                if current_value == &text {
                    output.print(&ActionResult {
                        success: true,
                        message: Some("text already matches, no action taken".into()),
                        path: None,
                        data: None,
                    });
                    return Ok(());
                }
            }

            let result = actions::type_into(platform, &chain, &text, false, timeout).await?;
            output.print(&result);
        }

        Command::Open { app, wait: do_wait } => {
            platform.open_application(&app).await?;

            if do_wait {
                let chain = SelectorChain::single(Selector::new().with_app(&app));
                let _ =
                    wait::poll_for_element(platform, &chain, timeout, Duration::from_millis(500))
                        .await?;
            }

            output.print(&ActionResult {
                success: true,
                message: Some(format!("opened '{app}'")),
                path: None,
                data: None,
            });
        }

        Command::Run {
            file,
            app: cli_app,
            pid,
            dry_run,
        } => {
            let cli_app = app_target(cli_app, pid);
            let contents =
                std::fs::read_to_string(&file).map_err(agent_computer_use_core::Error::Io)?;
            let wf: workflow::Workflow = serde_yaml::from_str(&contents).map_err(|e| {
                agent_computer_use_core::Error::PlatformError {
                    message: format!("invalid workflow YAML: {e}"),
                }
            })?;

            if dry_run {
                output.print(&serde_json::json!({
                    "valid": true,
                    "steps": wf.steps.len(),
                    "app": wf.app,
                }));
                return Ok(());
            }

            match workflow::execute(platform, &wf, cli_app.as_deref(), timeout).await {
                Ok(results) => output.print(&results),
                Err(e) => {
                    return Err(agent_computer_use_core::Error::PlatformError {
                        message: format!("workflow failed: {e}"),
                    }
                    .into());
                }
            }
        }

        Command::Observe {
            app,
            pid,
            depth,
            refresh,
        } => {
            let app = app_target(app, pid);
            let refresh_interval = Duration::from_secs_f64(refresh);
            observe::run_observe(platform, app, depth, refresh_interval).await?;
        }

        Command::Snapshot {
            app,
            pid,
            depth,
            interactive,
            compact,
        } => {
            let app = app_target(app, pid);
            let tree = platform.tree(app.as_deref(), depth).await?;
            let result = snapshot::create_snapshot(&tree, app.as_deref(), interactive, compact);
            snapshot::save_refs(&result.refs, app.as_deref())?;
            println!("{}", result.snapshot);

            if let Some(d) = depth {
                let ref_count = result.refs.len();
                eprintln!(
                    "hint: {} refs found at depth {} — use -d {} if elements are missing",
                    ref_count,
                    d,
                    d + 3
                );
            }
        }

        Command::Batch { bail } => {
            let results = batch::execute_batch(platform, output, timeout, bail).await?;
            output.print(&results);
        }

        Command::Screenshot { path, app, pid } => {
            let app = app_target(app, pid);
            let result = platform
                .perform(&Action::Screenshot {
                    path,
                    app: app.clone(),
                })
                .await?;
            output.print(&result);
        }

        Command::Windows { app, pid } => {
            let app = app_target(app, pid);
            let windows = platform.windows(app.as_deref()).await?;
            output.print(&windows);
        }

        Command::MoveWindow { app, pid, x, y } => {
            let app = app_target(app, pid);
            let app_name = app.ok_or_else(|| agent_computer_use_core::Error::PlatformError {
                message: "move-window requires --app or --pid".into(),
            })?;
            let moved = platform.move_window(&app_name, x, y).await?;
            output.print(&ActionResult {
                success: moved,
                message: Some(if moved {
                    format!("moved {app_name} to ({x}, {y})")
                } else {
                    "failed to move window".into()
                }),
                path: None,
                data: None,
            });
        }

        Command::ResizeWindow {
            app,
            pid,
            width,
            height,
        } => {
            let app = app_target(app, pid);
            let app_name = app.ok_or_else(|| agent_computer_use_core::Error::PlatformError {
                message: "resize-window requires --app or --pid".into(),
            })?;
            let resized = platform.resize_window(&app_name, width, height).await?;
            output.print(&ActionResult {
                success: resized,
                message: Some(if resized {
                    format!("resized {app_name} to ({width}, {height})")
                } else {
                    "failed to resize window".into()
                }),
                path: None,
                data: None,
            });
        }

        Command::Focused => {
            let node = platform.focused().await?;
            output.print(&node);
        }

        Command::Text { app, pid } => {
            let app = app_target(app, pid);
            let text = platform.text(app.as_deref()).await?;
            println!("{text}");
        }

        Command::Apps => {
            let apps = platform.applications().await?;
            output.print(&apps);
        }

        Command::Setup { yes, mode, scope } => {
            crate::setup::run(yes, mode, scope)?;
            let _ = output;
        }

        Command::Completions { shell } => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(
                shell,
                &mut cmd,
                "agent-computer-use",
                &mut std::io::stdout(),
            );
        }

        Command::CheckPermissions => {
            let granted = platform.check_permissions().await?;
            if granted {
                println!("Accessibility permissions: granted");
            } else {
                eprintln!("Accessibility permissions: NOT granted");
                eprintln!();
                eprintln!("Go to: System Settings > Privacy & Security > Accessibility");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn parse_mouse_button(s: Option<&str>) -> MouseButton {
    match s {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

fn app_target(app: Option<String>, pid: Option<u32>) -> Option<String> {
    pid.map(|pid| format!("pid:{pid}")).or(app)
}

async fn output_or_expect(
    platform: &dyn Platform,
    result: ActionResult,
    expect: Option<String>,
    timeout: Duration,
    output: &Output,
) -> Result<(), RunError> {
    match expect {
        Some(expect_dsl) => {
            let chain = actions::parse_selector(&expect_dsl).map_err(RunError::Core)?;
            match wait::poll_for_element(platform, &chain, timeout, actions::POLL_INTERVAL).await {
                Ok(element) => {
                    output.print(&ExpectResult {
                        success: result.success,
                        message: result.message,
                        expect: ExpectOutcome {
                            met: true,
                            message: None,
                            element: Some(element),
                        },
                    });
                    Ok(())
                }
                Err(agent_computer_use_core::Error::Timeout { seconds, message }) => {
                    Err(RunError::ExpectFailed {
                        action_result: result,
                        message: format!("expect timed out after {seconds}s: {message}"),
                    })
                }
                Err(e) => Err(RunError::Core(e)),
            }
        }
        None => {
            output.print(&result);
            Ok(())
        }
    }
}
