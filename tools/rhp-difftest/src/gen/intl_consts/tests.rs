use super::constants;

#[test]
fn guards_wrap_only_constants() {
    let stub = "<?php\n/**\n * @var int\n * @cvalue A_C\n */\nconst A = UNKNOWN;\n\
                    #ifdef X\n/**\n * @var string\n * @cvalue B_C\n */\nconst B = UNKNOWN;\n#endif\n\
                    #if Y\nfunction f(): void {}\n#endif\n\
                    class C {\n    /** @cvalue Z */\n    public const int D = UNKNOWN;\n}\n";
    assert_eq!(
        constants(stub).unwrap(),
        "RHP_INTL_INT(\"A\", A_C)\n#ifdef X\nRHP_INTL_STR(\"B\", B_C)\n#endif\n"
    );
}

#[test]
fn class_constants_keep_guards_and_attributes() {
    let stub = "<?php\nconst G = UNKNOWN;\nclass C\n{\n    /** @cvalue A_C */\n    public const int A = UNKNOWN;\n\
                #if V >= 69\n    /** @cvalue B_C */\n    public const string B = UNKNOWN;\n#endif\n\
                    public const null N = null;\n    /**\n     * @cvalue D_C\n     */\n\
                    #[\\Deprecated(since: '8.3')]\n    public const int D = UNKNOWN;\n\
                    public function f(): void {}\n}\nclass E\n{\n    /** @cvalue X */\n    public const int X = UNKNOWN;\n}\n";
    assert_eq!(
        super::class_constants(stub, "C").unwrap(),
        "RHP_INTL_CLASS_INT(\"C\", \"A\", A_C, \"\")\n#if V >= 69\n\
         RHP_INTL_CLASS_STR(\"C\", \"B\", B_C, \"\")\n#endif\n\
         RHP_INTL_CLASS_NULL(\"C\", \"N\", 0, \"\")\n\
         RHP_INTL_CLASS_INT(\"C\", \"D\", D_C, \"#[\\\\Deprecated(since: '8.3')]\")\n"
    );
}
