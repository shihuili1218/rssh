import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

// RSSH IntelliJ plugin — hosts the rssh web frontend in a JCEF tool window and
// spawns the self-contained `rssh-server` binary behind it.
//
// Versions below are a best-effort starting point; bump to match your target
// IDE. Build:  ./gradlew runIde      (set RSSH_SERVER_BIN to a built rssh-server)
// Package:     ./gradlew buildPlugin (zip → build/distributions, install-from-disk)

plugins {
    kotlin("jvm") version "2.0.21"
    id("org.jetbrains.intellij.platform") version "2.1.0"
}

group = "sh.rssh"
version = "0.1.6" // 0.1.6: title-bar Close (✕) button stops the rssh-server + browser and hides the tool window; reopening from the sidebar respawns a fresh session (server/browser re-rooted under a per-session Disposable, relaunch driven off toolWindowShown)
// 0.1.5: WS read loop breaks (not `?`-returns) on abnormal close so the cleanup block always runs — closes the last Finding-3 hole codex found (TCP reset / JCEF kill skipped shutdown+waiter-clear, leaking parked SSH prompt workers)

repositories {
    mavenCentral()
    intellijPlatform { defaultRepositories() }
}

dependencies {
    intellijPlatform {
        // Local dev builds against the installed IDE — no multi-hundred-MB SDK
        // download, and it's exactly the runtime the plugin will load into.
        // Override the path with -PrsshIde=/path/to/AnotherIDE.app. CI runners
        // have no local IDE, so they pass -PrsshIdeVersion (e.g. 2024.2) to
        // download the matching SDK instead.
        val ideVersion = providers.gradleProperty("rsshIdeVersion").orNull
        if (ideVersion != null) {
            create("IC", ideVersion)
        } else {
            local(providers.gradleProperty("rsshIde").orElse("/Applications/IntelliJ IDEA CE.app"))
        }
        instrumentationTools()
    }
}

intellijPlatform {
    // We ship no Settings UI (just a tool window), so skip the searchable-options
    // index build — that task spins up a headless IDE and is brittle against newer
    // target IDEs; it adds nothing for this plugin.
    buildSearchableOptions = false

    pluginConfiguration {
        ideaVersion {
            sinceBuild = "242"
            untilBuild = provider { null } // don't pin an upper bound
        }
    }
}

kotlin {
    // 2024.2+ (build 242+, the sinceBuild floor) all run JBR 21, so target 21 —
    // matches both the shipped artifact and local dev on 2026.1 / build 261.
    jvmToolchain(21)
}

// The local IDE's platform classes are compiled with a newer Kotlin than the
// kotlin-gradle-plugin here, so the compiler would flag them as "incompatible"
// (and, in 2.0.21, crash in the diagnostic reporter). Tolerate the newer
// metadata — we only call platform APIs, we don't depend on their internals.
tasks.withType<KotlinCompile>().configureEach {
    compilerOptions {
        freeCompilerArgs.addAll("-Xskip-metadata-version-check", "-Xskip-prerelease-check")
    }
}

// Release packaging: drop a per-OS `rssh-server` into resources/bin so the zip is
// self-contained. CI passes -PrsshServerBin to the freshly-built per-target binary
// (release.yml / pre-release.yml build one plugin zip per OS); local dev falls back
// to the host release path, or skips bundling and sets RSSH_SERVER_BIN at runtime.
val serverBin = file(
    providers.gradleProperty("rsshServerBin")
        .orElse(layout.projectDirectory.file("../src-tauri/target/release/rssh-server").asFile.path)
        .get()
)
tasks.processResources {
    if (serverBin.exists()) {
        // The plugin resolves "rssh-server.exe" on Windows, the bare name elsewhere;
        // name the bundled resource to match the binary we were handed.
        val bundledName = if (serverBin.name.endsWith(".exe")) "rssh-server.exe" else "rssh-server"
        from(serverBin) { into("bin"); rename { bundledName } }
    } else {
        logger.lifecycle("rssh-server binary not found at ${serverBin.path}; plugin will rely on RSSH_SERVER_BIN at runtime.")
    }
}
