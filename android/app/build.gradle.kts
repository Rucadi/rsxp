plugins { id("com.android.application") }

android {
    namespace = "org.rsxp.engine"
    compileSdk = 35
    defaultConfig {
        applicationId = "org.rsxp.engine"
        minSdk = 23
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    sourceSets["main"].manifest.srcFile("src/main/AndroidManifest.xml")
    sourceSets["main"].jniLibs.srcDir("src/main/jniLibs")
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/rubyAssets"))
}

val stageRubyAssets by tasks.registering(Sync::class) {
    listOf("arm64-v8a", "x86_64").forEach { abi ->
        from(rootProject.projectDir.resolve("ruby/$abi/stdlib")) { into("ruby/$abi/3.4.0") }
    }
    into(layout.buildDirectory.dir("generated/rubyAssets"))
}

tasks.named("preBuild").configure { dependsOn(stageRubyAssets) }
