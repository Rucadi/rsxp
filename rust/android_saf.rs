// SPDX-License-Identifier: GPL-2.0-or-later
//! SAF-backed game files. Android grants access through a persisted tree URI,
//! so all traversal and reads go through the Java ContentResolver adapter.
use android_activity::AndroidApp;
use jni::{
    objects::{JByteArray, JClass, JObject, JString, JValue},
    signature::RuntimeMethodSignature,
    strings::JNIString,
    Env, JavaVM,
};

fn with_activity<T>(
    app: &AndroidApp,
    callback: impl for<'local> FnOnce(&mut Env<'local>, &JObject<'local>) -> jni::errors::Result<T>,
) -> Result<T, String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    vm.attach_current_thread_for_scope(|env| {
        // SAFETY: AndroidApp owns the global Activity reference; JObject does
        // not delete it when dropped, and the callback does not retain it.
        let activity = unsafe { JObject::from_raw(env, raw_activity) };
        callback(env, &activity)
    })
    .map_err(|error| format!("Android SAF call failed: {error}"))
}

fn call_path<T>(
    app: &AndroidApp,
    method: &'static str,
    signature: &'static str,
    path: &str,
    convert: impl for<'local> FnOnce(
        &mut Env<'local>,
        jni::objects::JValueOwned<'local>,
    ) -> jni::errors::Result<T>,
) -> Result<T, String> {
    with_activity(app, |env, activity| {
        let path = env.new_string(path)?;
        let class = helper_class(env, activity)?;
        let name = JNIString::new(method);
        let signature = RuntimeMethodSignature::from_str(signature)
            .map_err(|error| jni::errors::Error::ParseFailed(error.to_string()))?;
        let value = env.call_static_method(
            &class,
            &name,
            signature.method_signature(),
            &[JValue::Object(activity), JValue::Object(&path)],
        )?;
        convert(env, value)
    })
}

fn helper_class<'local>(
    env: &mut Env<'local>,
    activity: &JObject<'local>,
) -> jni::errors::Result<JClass<'local>> {
    let loader = call_method(
        env,
        activity,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
        &[],
    )?
    .l()?;
    let name = env.new_string("org.rsxp.engine.SafGameFiles")?;
    let class = call_method(
        env,
        &loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&name)],
    )?
    .l()?;
    JClass::cast_local(env, class)
}

fn call_method<'local>(
    env: &mut Env<'local>,
    object: &JObject<'_>,
    method: &'static str,
    signature: &'static str,
    args: &[JValue<'_>],
) -> jni::errors::Result<jni::objects::JValueOwned<'local>> {
    let name = JNIString::new(method);
    let signature = RuntimeMethodSignature::from_str(signature)
        .map_err(|error| jni::errors::Error::ParseFailed(error.to_string()))?;
    env.call_method(object, &name, signature.method_signature(), args)
}

pub fn read_file(app: &AndroidApp, path: &str) -> Result<Option<Vec<u8>>, String> {
    call_path(
        app,
        "readFile",
        "(Landroid/app/Activity;Ljava/lang/String;)[B",
        path,
        |env, value| {
            let value = value.l()?;
            if value.is_null() {
                return Ok(None);
            }
            let array = JByteArray::cast_local(env, value)?;
            Ok(Some(env.convert_byte_array(&array)?))
        },
    )
}

pub fn file_exists(app: &AndroidApp, path: &str) -> Result<bool, String> {
    call_path(
        app,
        "fileExists",
        "(Landroid/app/Activity;Ljava/lang/String;)Z",
        path,
        |_env, value| value.z(),
    )
}

pub fn is_directory(app: &AndroidApp, path: &str) -> Result<bool, String> {
    call_path(
        app,
        "isDirectory",
        "(Landroid/app/Activity;Ljava/lang/String;)Z",
        path,
        |_env, value| value.z(),
    )
}

pub fn read_dir(app: &AndroidApp, path: &str) -> Result<Option<Vec<String>>, String> {
    call_path(
        app,
        "list",
        "(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;",
        path,
        |env, value| {
            let value = value.l()?;
            if value.is_null() {
                return Ok(None);
            }
            let value = JString::cast_local(env, value)?;
            let value = value.try_to_string(env)?;
            Ok(Some(
                value
                    .split('\0')
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ))
        },
    )
}

pub fn game_id(app: &AndroidApp) -> Result<String, String> {
    with_activity(app, |env, activity| {
        let class = helper_class(env, activity)?;
        let name = JNIString::new("gameId");
        let signature =
            RuntimeMethodSignature::from_str("(Landroid/app/Activity;)Ljava/lang/String;")
                .map_err(|error| jni::errors::Error::ParseFailed(error.to_string()))?;
        let value = env
            .call_static_method(
                &class,
                &name,
                signature.method_signature(),
                &[JValue::Object(activity)],
            )?
            .l()?;
        let value = JString::cast_local(env, value)?;
        value.try_to_string(env)
    })
}
