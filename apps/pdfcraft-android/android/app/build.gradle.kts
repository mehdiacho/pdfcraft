// PdfCraft for Android. The app is the Rust crate one level up (`apps/pdfcraft-android`),
// built with cargo-ndk into a `.so` per ABI before every build; this module adds the
// GameActivity it runs in and the Kotlin glue for the system pickers and intents.

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val workspaceDir = rootDir.resolve("../../..").canonicalFile
val rustLibsDir = layout.buildDirectory.dir("rustJniLibs")
val generatedResDir = layout.buildDirectory.dir("generated/pdfcraftRes")

// `-Prust.abis=arm64-v8a,x86_64` picks the ABIs (default: phones plus the x86_64 emulator);
// `-Prust.profile=dev` builds the Rust side without optimisations (much slower to run).
val rustAbis = (findProperty("rust.abis") as String? ?: "arm64-v8a,x86_64").split(",").map { it.trim() }.filter { it.isNotEmpty() }
val rustProfile = findProperty("rust.profile") as String? ?: "release"

android {
    namespace = "com.mehdiacho.pdfcraft"
    compileSdk = 35
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "com.mehdiacho.pdfcraft"
        // wgpu needs Vulkan 1.1 or GLES 3; Android 9 is where both are dependable.
        minSdk = 28
        targetSdk = 35
        versionCode = 1
        versionName = "0.8.0"
        ndk { abiFilters += rustAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    sourceSets["main"].jniLibs.srcDir(rustLibsDir)
    sourceSets["main"].res.srcDir(generatedResDir)

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
            // cargo-ndk copies every .so in the target folder, including a few dependencies
            // build as dylibs (hash-suffixed names); the app links none of them.
            excludes += "**/lib*-*.so"
        }
    }
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

dependencies {
    // The Java side of android-activity's GameActivity glue; keep in step with the
    // android-activity crate (README: "Game Activity Library Version"). No prefab.
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.core:core-ktx:1.16.0")
}

// The launcher icon is rendered from the app's own icon (assets/app-icon) at build time,
// so no copy of it is committed here.
val launcherIcon by tasks.registering(Copy::class) {
    val src = workspaceDir.resolve("assets/app-icon/hicolor")
    val densities = mapOf("mdpi" to "48x48", "hdpi" to "64x64", "xhdpi" to "128x128", "xxhdpi" to "256x256", "xxxhdpi" to "256x256")
    densities.forEach { (density, size) ->
        from(src.resolve("$size/apps")) {
            include("ai.storyteller.pdfcraft.png")
            rename { "ic_launcher.png" }
            into("mipmap-$density")
        }
    }
    into(generatedResDir)
}

val cargoBuild by tasks.registering(Exec::class) {
    description = "Builds libpdfcraft_android.so for each ABI with cargo-ndk."
    workingDir = workspaceDir
    val args = mutableListOf("cargo", "ndk", "--platform", "28", "-o", rustLibsDir.get().asFile.absolutePath)
    rustAbis.forEach { args += listOf("-t", it) }
    args += listOf("build", "-p", "pdfcraft-android", "--lib")
    if (rustProfile == "release") args += "--release" else args += listOf("--profile", rustProfile)
    commandLine(args)
}

tasks.named("preBuild") { dependsOn(cargoBuild, launcherIcon) }
