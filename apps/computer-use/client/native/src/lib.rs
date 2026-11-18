//! Computer Use's native code in the app: a phone's own screen, which a server acting for its person uses through
//! the app while it runs, as `computer-use_<action>` asked of the device (`client::DeviceTools`). On Android it's the
//! app's accessibility service (`clients/app/gen/android/.../ComputerUse.kt`), which its person turns on in Settings
//! → Accessibility. Elsewhere a device has none: a computer's screen is Computer Use's service on its server, and an
//! iPhone lets no app use its screen.

use std::sync::Arc;

use client::DeviceTools;

/// The tools of this device's own that Computer Use answers: on Android its screen, and elsewhere none.
pub fn tools() -> Vec<Arc<dyn DeviceTools>> {
    #[cfg(target_os = "android")]
    return vec![Arc::new(android::Phone)];
    #[cfg(not(target_os = "android"))]
    return Vec::new();
}

/// Whether this phone's person has turned Computer Use on, `{"on": bool}`, for Computer Use's page on the phone:
/// nothing on a device that has none, or a phone that can't, before Android 11.
pub fn phone() -> Option<serde_json::Value> {
    #[cfg(target_os = "android")]
    return android::act("screen", &serde_json::Value::Null).ok().map(|screen| serde_json::json!({"on": screen["structuredContent"]["on"]}));
    #[cfg(not(target_os = "android"))]
    return None;
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;

    use anyhow::{Context, Result, bail};
    use client::DeviceTools;
    use futures_util::future::BoxFuture;
    use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue, JValueOwned};
    use jni::{JNIEnv, JavaVM};
    use serde_json::{Value, json};

    /// This phone's screen, as its accessibility service sees and touches it.
    pub struct Phone;

    impl DeviceTools for Phone {
        fn app(&self) -> &'static str {
            "computer-use"
        }

        fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value>> {
            let tool = tool.to_owned();
            Box::pin(async move { tokio::task::spawn_blocking(move || act(&tool, &arguments)).await? })
        }
    }

    /// Does `tool` on the screen, on a thread of its own, which waits while Android does it.
    pub fn act(tool: &str, inputs: &Value) -> Result<Value> {
        let int = |name: &str| inputs[name].as_i64().map(|n| n as i32).with_context(|| format!("give {name}"));
        let text = |name: &str| inputs[name].as_str().map(str::to_owned).with_context(|| format!("give {name}"));
        let done = |done: String| json!({"content": [{"type": "text", "text": done}], "structuredContent": {"done": done}});
        Ok(match tool {
            "screen" => {
                let screen: Value = serde_json::from_str(&with(|env, class, context| {
                    let said = env.call_static_method(class, "screen", "(Landroid/content/Context;)Ljava/lang/String;", &[context.into()])?;
                    string(env, said.l()?)
                })?)?;
                let screen = json!({"name": client::name(), "width": screen["width"], "height": screen["height"], "on": screen["on"]});
                json!({"content": [{"type": "text", "text": screen.to_string()}], "structuredContent": screen})
            }
            "screenshot" => {
                let png = with(|env, class, context| {
                    let png = env.call_static_method(class, "screenshot", "(Landroid/content/Context;)[B", &[context.into()])?;
                    env.convert_byte_array(JByteArray::from(png.l()?))
                })?;
                let data = data_encoding::BASE64.encode(&png);
                json!({"content": [{"type": "image", "mimeType": "image/png", "data": data}]})
            }
            "click" => {
                let button = inputs["button"].as_str().unwrap_or("left").to_owned();
                done(say("click", "IILjava/lang/String;", vec![Int(int("x")?), Int(int("y")?), Text(button)])?)
            }
            "double_click" => done(say("doubleClick", "II", vec![Int(int("x")?), Int(int("y")?)])?),
            "move" => bail!("A phone has no pointer to move, and nothing on it shows what's under one: tap with click, or drag."),
            "drag" => {
                let path = inputs["path"].as_array().context("give path")?.iter().flat_map(|at| [at["x"].as_i64(), at["y"].as_i64()]);
                let path = path.map(|n| n.map(|n| n as i32).context("give each point's x and y")).collect::<Result<_>>()?;
                done(say("drag", "[I", vec![Ints(path)])?)
            }
            "scroll" => {
                let amount = inputs["amount"].as_i64().unwrap_or(5) as i32;
                let args = vec![Int(int("x")?), Int(int("y")?), Text(text("direction")?), Int(amount)];
                done(say("scroll", "IILjava/lang/String;I", args)?)
            }
            "type" => done(say("type", "Ljava/lang/String;", vec![Text(text("text")?)])?),
            "key" => done(say("key", "Ljava/lang/String;", vec![Text(text("keys")?)])?),
            other => bail!("A phone's screen has no {other}"),
        })
    }

    /// What an action is given, after the app.
    enum Arg {
        Int(i32),
        Text(String),
        Ints(Vec<i32>),
    }
    use Arg::*;

    /// Calls the service's `method`, which takes the app and then what `takes` says, with `args`, and gives what it
    /// said it did.
    fn say(method: &str, takes: &str, args: Vec<Arg>) -> Result<String> {
        with(|env, class, context| {
            let mut given = Vec::new();
            for arg in args {
                given.push(match arg {
                    Int(n) => JValueOwned::Int(n),
                    Text(text) => JValueOwned::Object(env.new_string(text)?.into()),
                    Ints(ints) => {
                        let array = env.new_int_array(ints.len() as i32)?;
                        env.set_int_array_region(&array, 0, &ints)?;
                        JValueOwned::Object(array.into())
                    }
                });
            }
            let all: Vec<JValue> = [JValue::from(context)].into_iter().chain(given.iter().map(JValueOwned::borrow)).collect();
            let signature = format!("(Landroid/content/Context;{takes})Ljava/lang/String;");
            let said = env.call_static_method(class, method, signature, &all)?;
            string(env, said.l()?)
        })
    }

    fn string(env: &mut JNIEnv, said: JObject) -> jni::errors::Result<String> {
        if said.is_null() {
            return Err(jni::errors::Error::NullPtr("nothing was said"));
        }
        Ok(env.get_string(&JString::from(said))?.into())
    }

    /// Runs `run` with the service's class and the app, on this thread attached to the app's Java VM, and gives what
    /// it threw as an error, with its message. Tauri's window on Android gives what it runs in to `ndk_context` as it
    /// starts, and the class is loaded by the app's own class loader, as a thread of Rust's has the system's.
    fn with<T>(run: impl FnOnce(&mut JNIEnv, &JClass, &JObject) -> jni::errors::Result<T>) -> Result<T> {
        static CLASS: OnceLock<GlobalRef> = OnceLock::new();
        let android = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(android.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let context = unsafe { JObject::from_raw(android.context().cast()) };
        let class = match CLASS.get() {
            Some(class) => class.clone(),
            None => {
                let loader = env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
                let name = JObject::from(env.new_string("app.theone.client.ComputerUse")?);
                let class = env.call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[(&name).into()])?.l()?;
                CLASS.get_or_init(|| env.new_global_ref(class).expect("a class has a global reference")).clone()
            }
        };
        let ran = run(&mut env, <&JClass>::from(class.as_obj()), &context);
        match ran {
            Err(jni::errors::Error::JavaException) => {
                let thrown = env.exception_occurred()?;
                env.exception_clear()?;
                let message = env.call_method(&thrown, "getMessage", "()Ljava/lang/String;", &[])?.l()?;
                bail!("{}", string(&mut env, message).unwrap_or_else(|_| "Android couldn't do that".into()))
            }
            ran => Ok(ran?),
        }
    }
}
