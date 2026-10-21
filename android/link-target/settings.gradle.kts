// A throwaway test app, not part of Kin: an app that owns the web address https://kin-link-test.example/..., like YouTube owns youtube.com, so a tap
// on a link notification can be seen opening "the app that owns the link" instead of the browser. Build it with:  ..\gradlew.bat -p link-target assembleDebug
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "link-target"
