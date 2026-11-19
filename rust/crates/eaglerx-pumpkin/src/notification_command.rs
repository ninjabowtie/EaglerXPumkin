//! Pumpkin command adapter. Host permission checks happen before dispatch;
//! recheck the same permission before touching gateway state.
use crate::{
    notifications::{DismissNotification, Notification},
    runtime::Runtime,
};
use pumpkin_plugin_api::{
    Context, Server,
    command::{
        Arg, ArgumentType, Command, CommandError, CommandNode, CommandSender, ConsumedArgs,
        StringType,
    },
    commands::CommandHandler,
    permission::{Permission, PermissionDefault, PermissionLevel},
    text::TextComponent,
};
use std::sync::{Arc, Mutex};

const PERMISSION: &str = "eaglerx-pumpkin:command.notify";

pub fn register(context: &Context, runtime: Arc<Mutex<Option<Runtime>>>) -> Result<(), String> {
    context.register_permission(&Permission {
        node: PERMISSION.into(),
        description: "Send dynamic notification badges to Eagler clients".into(),
        default: PermissionDefault::Op(PermissionLevel::Two),
        children: vec![],
    })?;
    for (name, dismiss) in [("eaglernotify", false), ("eaglernotifyhide", true)] {
        let command = Command::new(
            &[name.into()],
            "Control an Eagler badge: <username|*> <JSON>",
        );
        let make_json = |broadcast| {
            CommandNode::argument("notification", &ArgumentType::String(StringType::Greedy))
                .execute(Handler {
                    runtime: Arc::clone(&runtime),
                    dismiss,
                    broadcast,
                })
        };
        let broadcast = CommandNode::literal("*").then(make_json(true));
        let target = CommandNode::argument("target", &ArgumentType::String(StringType::SingleWord))
            .then(make_json(false));
        context.register_command(command.then(broadcast).then(target), PERMISSION);
    }

    Ok(())
}

struct Handler {
    broadcast: bool,
    dismiss: bool,
    runtime: Arc<Mutex<Option<Runtime>>>,
}

fn failure(message: &str) -> CommandError {
    CommandError::CommandFailed(TextComponent::text(message))
}

impl CommandHandler for Handler {
    fn handle(
        &self,
        sender: CommandSender,
        server: Server,
        args: ConsumedArgs,
    ) -> Result<i32, CommandError> {
        if !sender.has_permission(&server, PERMISSION) {
            return Err(CommandError::PermissionDenied);
        }
        let target = if self.broadcast {
            "*".into()
        } else if let Arg::Simple(target) = args.get_value("target") {
            target
        } else {
            return Err(failure("Expected username or *"));
        };
        let Arg::Simple(json) = args.get_value("notification") else {
            return Err(failure("Expected notification JSON"));
        };
        let request = if self.dismiss {
            Request::Hide(DismissNotification::from_json(&json).map_err(|e| failure(&e))?)
        } else {
            Request::Show(Notification::from_json(&json).map_err(|e| failure(&e))?)
        };
        // Release the lock before invoking host feedback APIs.
        let queued = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| failure("Gateway lock unavailable"))?;
            let runtime = runtime
                .as_mut()
                .ok_or_else(|| failure("Gateway is unloaded"))?;
            match &request {
                Request::Show(request) => runtime.notify(&target, request),
                Request::Hide(request) => runtime.dismiss_notification(&target, request),
            }
            .map_err(|e| failure(&e))?
        };
        sender.send_message(TextComponent::text(&format!(
            "Badge action queued for {queued} Eagler connection(s)"
        )));
        Ok(queued as i32)
    }
}

enum Request {
    Show(Notification),
    Hide(DismissNotification),
}
