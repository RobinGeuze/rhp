opt -dprecision=3
opt --define
opt serialize_precision=5
opt -r
opt echo ini_get("precision"), " ", ini_get("serialize_precision"), "\n";
