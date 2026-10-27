struct Receipt: Codable {
    let pid: Int32
    let startTime: UInt64
    let bundle: String
    let executable: String
    let identifier: String
}
// Both releases use the same fixed installed bundle folder. Static, so it is
// ready wherever top-level code reads it.
enum ProductBundle {
    static let earlier = "/Applications/XTrace.app"
    static let current = "/Applications/XTrace.app"
}
