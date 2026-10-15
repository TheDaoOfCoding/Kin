// A throwaway test app, not part of Kin: a second app that shares files to Kin, to test what the share sheet really does
// (a foreign provider, a temporary read grant, a source app that dies). Build it with:  ..\gradlew.bat -p share-source assembleDebug
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
rootProject.name = "share-source"
