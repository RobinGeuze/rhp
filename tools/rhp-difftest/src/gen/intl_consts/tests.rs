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
