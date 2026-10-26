struct Receipt: Codable {
    let pid: Int32
    let startTime: UInt64
    let bundle: String
    let executable: String
    let identifier: String
}
// The installed app's two fixed bundle folders: "XTrace Desktop.app" through
// release 0.1.7, "XTrace.app" since (an updated earlier copy renames itself on
// a normal quit). Static, so it is ready wherever top-level code reads it.
enum ProductBundle {
    static let earlier = "/Applications/XTrace Desktop.app"
    static let current = "/Applications/XTrace.app"
}
