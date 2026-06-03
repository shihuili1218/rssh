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
        // Build against the locally-installed IDE — no multi-hundred-MB SDK
        // download, and it's exactly the runtime the plugin will load into.
        // Override with -PrsshIde=/path/to/AnotherIDE.app if yours differs.
        local(providers.gradleProperty("rsshIde").orElse("/Applications/IntelliJ IDEA CE.app"))
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
    // The local IDE (2026.1 / build 261) runs on JBR 21, so target 21 — also
    // matches the JDK building this. For an older target IDE (242 runs JBR 17),
    // drop this to 17 and build with a JDK 17.
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
// self-contained. For a multi-OS release, run this on each platform (or in CI) and
// merge. Dev runs just set RSSH_SERVER_BIN instead.
val bundledServer = layout.projectDirectory.file("../src-tauri/target/release/rssh-server")
tasks.processResources {
    if (bundledServer.asFile.exists()) {
        from(bundledServer) { into("bin") }
    } else {
        logger.lifecycle("rssh-server release binary not found; plugin will rely on RSSH_SERVER_BIN at runtime.")
    }
}
