<?php
spl_autoload_register(function($c){ throw new Exception("t $c"); }); try { var_dump(class_exists("Foo")); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { var_dump(interface_exists("Bar")); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { var_dump(is_subclass_of("Baz", "X")); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { new Qux; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
class Ser implements JsonSerializable { function jsonSerialize(): mixed { throw new Exception("js"); } }
try { var_dump(json_encode(new Ser)); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { var_dump(json_encode([1, [new Ser]])); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
var_dump(class_exists("Foo", false));
try { var_dump(method_exists("M1", "x")); } catch (Exception $e) { echo "caught ", $e->getMessage(), "
"; }
try { var_dump(property_exists("M2", "x")); } catch (Exception $e) { echo "caught ", $e->getMessage(), "
"; }
try { var_dump(is_a("M3", "x", true)); } catch (Exception $e) { echo "caught ", $e->getMessage(), "
"; }
var_dump(is_a("M4", "x"));
