use super::constants;

#[test]
fn guards_and_attributes() {
    let stub = "<?php\n/** @cvalue NOPE */\nconst G = UNKNOWN;\n\
                    class ZipArchive implements Countable\n{\n\
                    \x20   /**\n     * @cvalue ZIP_CREATE\n     */\n    public const int CREATE = UNKNOWN;\n\
                    #ifdef ZIP_RDONLY\n    /**\n     * @cvalue ZIP_RDONLY\n     */\n    public const int RDONLY = UNKNOWN;\n#endif\n\
                    #ifdef X\n    /** @cvalue ZIP_FL_RECOMPRESS */\n    #[\\Deprecated(since: '8.3')]\n    public const int FL_RECOMPRESS = UNKNOWN;\n#endif\n\
                    #ifdef Y\n    public function f(): void {}\n#endif\n\
                    \x20   /** @cvalue LIBZIP_VERSION_STR */\n    public const string LIBZIP_VERSION = UNKNOWN;\n}\n";
    assert_eq!(
        constants(stub).unwrap(),
        "RHP_ZIP_INT(\"CREATE\", ZIP_CREATE, \"\")\n\
             #ifdef ZIP_RDONLY\nRHP_ZIP_INT(\"RDONLY\", ZIP_RDONLY, \"\")\n#endif\n\
             #ifdef X\nRHP_ZIP_INT(\"FL_RECOMPRESS\", ZIP_FL_RECOMPRESS, \"#[\\\\Deprecated(since: '8.3')]\")\n#endif\n\
             RHP_ZIP_STR(\"LIBZIP_VERSION\", LIBZIP_VERSION_STR, \"\")\n"
    );
}
