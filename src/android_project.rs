use std::collections::HashMap;
use std::fs::{copy, create_dir_all, read_to_string, write};
use std::path::Path;
use std::process::Command;

use fs_extra::{copy_items, dir::CopyOptions, remove_items};
use lazy_static::lazy_static;
use regex::{Regex, RegexBuilder};
use symlink::symlink_dir;

use crate::util::*;
use crate::BuildProfile;

pub fn build_sdl_for_android(targets: &Vec<&str>, profile: BuildProfile) {
  let p = Path::new(&*get_env_var("ANDROID_NDK_HOME")).join("ndk-build");

  assert!(Command::new(&p)
    .args([
      "NDK_PROJECT_PATH=.",
      "APP_BUILD_SCRIPT=./Android.mk",
      "APP_PLATFORM=android-19"
    ])
    .current_dir(&*get_env_var("SDL"))
    .status()
    .expect(&format!("Failed to execute command: {:?}", p))
    .success());

  for rust_name in targets {
    let android_name = get_target_android_name(rust_name);
    let rust_dir = Path::new("target")
      .join(rust_name)
      .join(profile.to_string())
      .join("deps");

    create_dir_all(rust_dir).expect("Unable to create target dir");
    let src = Path::new(&*get_env_var("SDL"))
      .join("libs")
      .join(android_name)
      .join("libSDL2.so");
    let dest = Path::new("target")
      .join(rust_name)
      .join(profile.to_string())
      .join("deps/libSDL2.so");
    copy(&src, &dest).expect(&format!(
      "Unable to copy SDL dependencies from {:?} to {:?}",
      src, dest
    ));
  }
}

pub fn get_target_android_name(rust_target_name: &str) -> &str {
  match rust_target_name {
    "aarch64-linux-android" => "arm64-v8a",
    "armv7-linux-androideabi" => "armeabi-v7a",
    "i686-linux-android" => "x86",
    "x86_64-linux-android" => "x86_64",
    _ => {
      panic!("Unknown target: {}", rust_target_name)
    }
  }
}

pub fn get_android_app_id(manifest_path: &Path) -> String {
  get_toml_string(
    manifest_path,
    vec!["package", "metadata", "android", "package_name"],
  )
  .unwrap_or("org.libsdl.app".to_string())
}

fn create_android_project(manifest_path: &Path, target_artifacts: &HashMap<String, String>) {
  let manifest_dir = manifest_path.parent().unwrap();

  let appid = get_android_app_id(manifest_path);
  let appname = get_toml_string(
    manifest_path,
    vec!["package", "metadata", "android", "title"],
  )
  .unwrap_or("Untitled".to_string());
  let app_icon = get_toml_string(
    manifest_path,
    vec!["package", "metadata", "android", "icon"],
  );

  // Copy template project from SDL
  copy_items(
    &[Path::new(&*get_env_var("SDL")).join("android-project")],
    Path::new(manifest_dir).join("target"),
    &CopyOptions::new().overwrite(true),
  )
  .unwrap();

  // Create main activity class
  let java_main_folder = manifest_dir
    .join("target/android-project/app/src/main/java")
    .join(str::replace(&appid, ".", "/"));
  create_dir_all(java_main_folder.clone()).unwrap();
  let main_class = "
		package $APP;

		import org.libsdl.app.SDLActivity;

		public class MainActivity extends SDLActivity {
		}
	";
  let main_class = str::replace(main_class, "$APP", &appid);
  write(java_main_folder.join("MainActivity.java"), &main_class).expect("Unable to write file");

  // Change project files
  change_android_project_file(
    manifest_dir,
    "app/src/main/AndroidManifest.xml",
    vec![("SDLActivity", "MainActivity"), ("org.libsdl.app", &*appid)],
  );

  change_android_project_file(
    manifest_dir,
    "app/build.gradle",
    vec![("org.libsdl.app", &*appid)],
  );

  change_android_project_file(
    manifest_dir,
    "app/src/main/res/values/strings.xml",
    vec![("Game", &*appname)],
  );

  //add permission entries
  for permission in get_toml_string_vec(
    manifest_path,
    ["package", "metadata", "android", "permissions"],
  )
  .unwrap_or(vec![])
  {
    println!("Adding permission entry for permission {}", permission);
    add_uses_permission_entry(manifest_dir, &permission);
  }

  // Remove C sources
  remove_items(&[manifest_dir.join("target/android-project/app/jni/src")]).unwrap();

  // Link SDL into project
  if !manifest_dir
    .join("target/android-project/app/jni/SDL")
    .is_dir()
  {
    symlink_dir(
      Path::new(&*get_env_var("SDL")),
      manifest_dir.join("target/android-project/app/jni/SDL"),
    )
    .unwrap();
  }

  // Copy libmain.so to all targets
  for (target, artifact) in target_artifacts {
    let target_android_name = get_target_android_name(target);
    //println!("{:?}",target);

    let android_dir = manifest_dir
      .join("target/android-project/app/src/main/jniLibs")
      .join(target_android_name);

    create_dir_all(&android_dir).unwrap();
    copy(artifact, android_dir.join("libmain.so")).unwrap();
  }

  //copy app icon
  if let Some(icon_path) = app_icon {
    let icon_path = manifest_dir.join(icon_path);
    for res in vec!["m", "h", "xh", "xxh", "xxxh"] {
      let dest = manifest_dir.join(format!(
        "target/android-project/app/src/main/res/mipmap-{}dpi/ic_launcher.png",
        res
      ));
      if let Err(e) = copy(&icon_path, &dest) {
        eprintln!(
          "Failed to copy icon from {:?} to {:?}: {}",
          icon_path, dest, e
        );
      }
    }
  }
}

lazy_static! {
  static ref MANIFEST_TAG_CONTENT_REGEX: Regex = RegexBuilder::new("<manifest.*?>(.*)</manifest>")
    .dot_matches_new_line(true)
    .build()
    .expect("invalid manifest tag regex");
}

fn add_uses_permission_entry(manifest_dir: &Path, permission: &str) {
  let path = manifest_dir.join("target/android-project/app/src/main/AndroidManifest.xml");
  let mut content = read_to_string(&path).expect(&format!("can't read manifest {:?}", path));
  let captures = MANIFEST_TAG_CONTENT_REGEX
    .captures(&content)
    .expect("can't find manifest tag content");
  let content_match = captures.get(1).expect("can't get content of manifest tag");
  let tag_content = content_match.as_str();

  let permission_entry = format!(
    "<uses-permission android:name=\"android.permission.{}\"/>",
    permission.to_uppercase()
  );
  if tag_content.contains(&permission_entry) {
    return;
  }

  content.insert_str(content_match.end(), &permission_entry);

  write(&path, &content).expect("can't write to manifest file");
}

fn change_android_project_file(
  manifest_dir: &Path,
  file_name: &str,
  replacements: Vec<(&str, &str)>,
) {
  let path = manifest_dir.join("target/android-project").join(file_name);
  let mut content = read_to_string(&path).expect(&format!("can't read project file: {:?}", path));

  for (from, to) in replacements {
    content = content.replace(from, to);
  }

  write(&path, &content).expect("unable to write file");
}

pub fn sign_android(manifest_path: &Path, ks_file: Option<String>, ks_pass: Option<String>) {
  let manifest_dir = manifest_path.parent().unwrap();
  let release_dir = manifest_dir.join("target/android-project/app/build/outputs/apk/release");
  //println!("{:?}",release_dir);

  // Find android build tools.
  let tool_paths =
    std::fs::read_dir(Path::new(&*get_env_var("ANDROID_HOME")).join("build-tools")).unwrap();
  let mut tool_paths: Vec<String> = tool_paths
    .map(|d| {
      d.unwrap()
        .path()
        .file_name()
        .unwrap()
        .to_os_string()
        .into_string()
        .unwrap()
    })
    .collect();
  tool_paths.sort();
  tool_paths.reverse();
  let tools_version = tool_paths[0].clone();
  println!("Using build-tools: {}", tools_version);

  // Determine key file. Generate if needed.
  let (key_file, key_pass) = if ks_file.is_some() {
    (ks_file.unwrap(), ks_pass.expect("Need keystore password"))
  } else {
    let key_path = release_dir.join("app-release.jks");
    if !key_path.exists() {
      println!("Generating keyfile...");
      assert!(Command::new("keytool")
        .arg("-genkey")
        .arg("-dname")
        .arg("CN=Unknown, OU=Unknown, O=Unknown, L=Unknown, S=Unknown, C=Unknown")
        .arg("-storepass")
        .arg("android")
        .arg("-keystore")
        .arg(key_path.clone())
        .arg("-keyalg")
        .arg("RSA")
        .arg("-keysize")
        .arg("2048")
        .arg("-validity")
        .arg("10000")
        .status()
        .unwrap()
        .success());
    }

    (
      key_path.into_os_string().into_string().unwrap(),
      "pass:android".to_string(),
    )
  };

  println!("Using keyfile: {}", key_file);

  // Run zipalign.
  let zipalign_path = Path::new(&*get_env_var("ANDROID_HOME"))
    .join("build-tools")
    .join(tools_version.clone())
    .join("zipalign");

  assert!(Command::new(zipalign_path)
    .arg("-v")
    .arg("-f")
    .arg("-p")
    .arg("4")
    .arg(release_dir.join("app-release-unsigned.apk"))
    .arg(release_dir.join("app-release-unsigned-aligned.apk"))
    .status()
    .unwrap()
    .success());

  // Run apksigner
  let apksigner_path = Path::new(&*get_env_var("ANDROID_HOME"))
    .join("build-tools")
    .join(tools_version.clone())
    .join("apksigner");

  assert!(Command::new(apksigner_path)
    .arg("sign")
    .arg("-ks")
    .arg(key_file)
    .arg("-ks-pass")
    .arg(key_pass)
    .arg("-out")
    .arg(release_dir.join("app-release.apk"))
    .arg(release_dir.join("app-release-unsigned-aligned.apk"))
    .status()
    .unwrap()
    .success());
}

// keytool -android blabla -genkey -v -keystore my-release-key.jks -keyalg RSA -keysize 2048 -validity 10000 -alias my-alias
// /home/micke/Android/Sdk/build-tools/30.0.3/zipalign -v -p 4 app-release-unsigned.apk app-release-unsigned-aligned.apk
// /home/micke/Android/Sdk/build-tools/30.0.3/apksigner sign -ks my-release-key.jks -ks-pass pass:android -out app-release.apk app-release-unsigned-aligned.apk

pub fn build_android_project(
  manifest_path: &Path,
  target_artifacts: &HashMap<String, String>,
  profile: BuildProfile,
  ks_file: Option<String>,
  ks_pass: Option<String>,
) {
  let manifest_dir = manifest_path.parent().unwrap();

  create_android_project(manifest_path, target_artifacts);

  let gradle_task = match profile {
    BuildProfile::Debug => "assembleDebug",
    BuildProfile::Release => "assembleRelease",
  };

  assert!(Command::new("./gradlew")
    .args([gradle_task])
    .current_dir(manifest_dir.join("./target/android-project"))
    .status()
    .unwrap()
    .success());

  if matches!(profile, BuildProfile::Release) {
    sign_android(manifest_path, ks_file, ks_pass);
  }
}

#[cfg(test)]
mod test {
  use crate::android_project::MANIFEST_TAG_CONTENT_REGEX;

  #[test]
  fn manifest_regex() {
    let mut manifest_file_content =
      String::from("<some header>\n<manifest option1\n\toption2>\n\t<hello world>\n</manifest>\n");
    let captures = MANIFEST_TAG_CONTENT_REGEX
      .captures(&manifest_file_content)
      .unwrap();
    let content_match = captures.get(1).unwrap();
    assert_eq!(content_match.as_str(), "\n\t<hello world>\n");

    manifest_file_content.insert_str(content_match.end(), "\t<permission>\n");

    let captures = MANIFEST_TAG_CONTENT_REGEX
      .captures(&manifest_file_content)
      .unwrap();
    let content_match = captures.get(1).unwrap();
    assert_eq!(
      content_match.as_str(),
      "\n\t<hello world>\n\t<permission>\n"
    );
  }
}
