//! 在固定的 SSH 连接上授权硬件读取，不复用或保存登录密码

use std::{rc::Rc, time::Duration};

use gpui_kit::component::{
    Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::{DialogAction, DialogClose, DialogFooter},
    form::{Field, Form},
    input::{Input, InputState},
};
use gpui_kit::*;

use crate::connection::ConnectionSecret;
use crate::i18n::t;
use crate::shared::{dismiss_form_error, form_error_notification};
use crate::terminal::{ExecOptions, ExecTarget, detailed_exec_answer};

use super::access;

type InstalledHandler = Rc<dyn Fn(&mut Window, &mut App)>;

struct AccessForm {
    target: ExecTarget,
    generation: u64,
    password: Entity<InputState>,
    busy: bool,
    on_installed: InstalledHandler,
}

impl AccessForm {
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        let Some(view) = self
            .target
            .view
            .upgrade()
            .filter(|view| view.read(cx).generation(cx) == self.generation)
        else {
            window.push_notification(form_error_notification(t!("monitor.access.changed")), cx);
            return false;
        };
        let input = ConnectionSecret::new(format!("{}\n", self.password.read(cx).value()));
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        let options = ExecOptions {
            timeout: Duration::from_secs(45),
            label: Some("monitor-authorize"),
            max_output_bytes: 16 * 1024,
        };
        let Some(handle) =
            view.read(cx)
                .exec_detailed_with_input(access::command(), options, input, cx)
        else {
            window.push_notification(form_error_notification(t!("monitor.access.changed")), cx);
            return false;
        };
        self.busy = true;
        dismiss_form_error(window, cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = detailed_exec_answer(handle, cx).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                if result.as_ref().is_ok_and(access::installed) {
                    dismiss_form_error(window, cx);
                    window.close_dialog(cx);
                    let on_installed = this.on_installed.clone();
                    window.defer(cx, move |window, cx| on_installed(window, cx));
                } else {
                    // 不展示可能由远端回显的认证内容
                    window.push_notification(
                        form_error_notification(t!("monitor.access.failed")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
        false
    }
}

impl Render for AccessForm {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Form::new().child(
            Field::new()
                .label(t!("monitor.access.password"))
                .description(t!("monitor.access.password_hint"))
                .child(
                    Input::new(&self.password)
                        .id("monitor-sudo-password")
                        .disabled(self.busy),
                ),
        )
    }
}

pub(crate) fn open_access_dialog(
    name: SharedString,
    target: ExecTarget,
    generation: u64,
    on_installed: InstalledHandler,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| AccessForm {
        target,
        generation,
        password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        busy: false,
        on_installed,
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let busy = form.read(cx).busy;
        dialog
            .title(t!("monitor.access.title", host = name))
            .overlay_closable(false)
            .close_button(!busy)
            .child(div().text_sm().child(t!("monitor.access.description")))
            .child(form.clone())
            .footer(
                DialogFooter::new()
                    .child(
                        DialogClose::new().trigger(move |button| {
                            button.label(t!("common.cancel")).disabled(busy)
                        }),
                    )
                    .child(
                        DialogAction::new().child(
                            Button::new("commit")
                                .primary()
                                .label(if busy {
                                    t!("monitor.access.working")
                                } else {
                                    t!("monitor.access.install")
                                })
                                .disabled(busy),
                        ),
                    ),
            )
            .on_ok({
                let form = form.clone();
                move |_, window, cx| form.update(cx, |form, cx| form.commit(window, cx))
            })
            .on_cancel({
                let form = form.clone();
                move |_, _, cx| !form.read(cx).busy
            })
            .on_close(|_, window, cx| dismiss_form_error(window, cx))
    });
}
