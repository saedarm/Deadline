// Gradle module for Deadline's Android reminders. `dx` copies this folder into the
// generated Android project as a library and links it in; you never open it in
// Android Studio unless you want to.
plugins {
    id("com.android.library") version "8.4.2"
    kotlin("android") version "1.9.24"
}

android {
    namespace = "com.saedarm.deadline.reminders"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        getByName("release") { isMinifyEnabled = false }
        getByName("debug") { isMinifyEnabled = false }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.work:work-runtime-ktx:2.9.1")
}
