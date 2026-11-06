//! ext/standard/credits.c: `phpcredits()` and phpinfo's credits, as text
//! (the CLI's phpinfo_as_text). The SAPI and module lists are php-src's
//! credits_sapi.h and credits_ext.h.

use rhp_values::ops::deref;
use rhp_values::{conv, Value};

use crate::ext::standard::info::Info;
use crate::{Host, Thrown};

pub(crate) const CREDITS_GROUP: i64 = 1 << 0;
pub(crate) const CREDITS_GENERAL: i64 = 1 << 1;
pub(crate) const CREDITS_SAPI: i64 = 1 << 2;
pub(crate) const CREDITS_MODULES: i64 = 1 << 3;
pub(crate) const CREDITS_DOCS: i64 = 1 << 4;
pub(crate) const CREDITS_FULLPAGE: i64 = 1 << 5;
pub(crate) const CREDITS_QA: i64 = 1 << 6;
pub(crate) const CREDITS_WEB: i64 = 1 << 7;
pub(crate) const CREDITS_ALL: i64 = 0xFFFF_FFFF;

const SAPI_CREDITS: &[(&str, &str)] = &[
    (
        "Apache 2 Handler",
        "Ian Holsman, Justin Erenkrantz (based on Apache 2 Filter code)",
    ),
    (
        "CGI / FastCGI",
        "Rasmus Lerdorf, Stig Bakken, Shane Caraveo, Dmitry Stogov",
    ),
    (
        "CLI",
        "Edin Kadribasic, Marcus Boerger, Johannes Schlueter, Moriyoshi Koizumi, Xinchen Hui",
    ),
    ("Embed", "Edin Kadribasic"),
    (
        "FastCGI Process Manager",
        "Andrei Nigmatulin, dreamcat4, Antony Dovgal, Jerome Loyet",
    ),
    ("litespeed", "George Wang"),
    ("phpdbg", "Felipe Pena, Joe Watkins, Bob Weinand"),
];

const EXT_CREDITS: &[(&str, &str)] = &[
    ("BC Math", "Andi Gutmans"),
    ("Bzip2", "Sterling Hughes"),
    ("Calendar", "Shane Caraveo, Colin Viebrock, Hartmut Holzgraefe, Wez Furlong"),
    ("COM and .Net", "Wez Furlong"),
    ("ctype", "Hartmut Holzgraefe"),
    ("cURL", "Sterling Hughes"),
    ("Date/Time Support", "Derick Rethans"),
    ("DB-LIB (MS SQL, Sybase)", "Wez Furlong, Frank M. Kromann, Adam Baratz"),
    ("DBA", "Sascha Schumann, Marcus Boerger"),
    ("DOM", "Christian Stocker, Rob Richards, Marcus Boerger, Nora Dossche"),
    ("enchant", "Pierre-Alain Joye, Ilia Alshanetsky"),
    ("EXIF", "Rasmus Lerdorf, Marcus Boerger"),
    ("FFI", "Dmitry Stogov"),
    ("fileinfo", "Ilia Alshanetsky, Pierre Alain Joye, Scott MacVicar, Derick Rethans, Anatol Belski"),
    ("Firebird driver for PDO", "Ard Biesheuvel"),
    ("FTP", "Stefan Esser, Andrew Skalski"),
    ("GD imaging", "Rasmus Lerdorf, Stig Bakken, Jim Winstead, Jouni Ahto, Ilia Alshanetsky, Pierre-Alain Joye, Marcus Boerger, Mark Randall"),
    ("GetText", "Alex Plotnick"),
    ("GNU GMP support", "Stanislav Malyshev"),
    ("Iconv", "Rui Hirokawa, Stig Bakken, Moriyoshi Koizumi"),
    ("Input Filter", "Rasmus Lerdorf, Derick Rethans, Pierre-Alain Joye, Ilia Alshanetsky"),
    ("Internationalization", "Ed Batutis, Vladimir Iordanov, Dmitry Lakhtyuk, Stanislav Malyshev, Vadim Savchuk, Kirti Velankar"),
    ("JSON", "Jakub Zelenka, Omar Kilani, Scott MacVicar"),
    ("LDAP", "Amitay Isaacs, Eric Warnke, Rasmus Lerdorf, Gerrit Thomson, Stig Venaas"),
    ("LIBXML", "Christian Stocker, Rob Richards, Marcus Boerger, Wez Furlong, Shane Caraveo"),
    ("Multibyte String Functions", "Tsukada Takuya, Rui Hirokawa"),
    ("MySQL driver for PDO", "George Schlossnagle, Wez Furlong, Ilia Alshanetsky, Johannes Schlueter"),
    ("MySQLi", "Zak Greant, Georg Richter, Andrey Hristov, Ulf Wendel"),
    ("MySQLnd", "Andrey Hristov, Ulf Wendel, Georg Richter, Johannes Schlüter"),
    ("ODBC driver for PDO", "Wez Furlong"),
    ("ODBC", "Stig Bakken, Andreas Karajannis, Frank M. Kromann, Daniel R. Kalowsky"),
    ("Opcache", "Andi Gutmans, Zeev Suraski, Stanislav Malyshev, Dmitry Stogov, Xinchen Hui"),
    ("OpenSSL", "Stig Venaas, Wez Furlong, Sascha Kettler, Scott MacVicar, Eliot Lear"),
    ("pcntl", "Jason Greene, Arnaud Le Blanc"),
    ("Perl Compatible Regexps", "Andrei Zmievski"),
    ("PHP Archive", "Gregory Beaver, Marcus Boerger"),
    ("PHP Data Objects", "Wez Furlong, Marcus Boerger, Sterling Hughes, George Schlossnagle, Ilia Alshanetsky"),
    ("PHP hash", "Sara Golemon, Rasmus Lerdorf, Stefan Esser, Michael Wallner, Scott MacVicar"),
    ("Posix", "Kristian Koehntopp"),
    ("PostgreSQL driver for PDO", "Edin Kadribasic, Ilia Alshanetsky"),
    ("PostgreSQL", "Jouni Ahto, Zeev Suraski, Yasuo Ohgaki, Chris Kings-Lynne"),
    ("random", "Go Kudo, Tim Düsterhus, Guilliam Xavier, Christoph M. Becker, Jakub Zelenka, Bob Weinand, Máté Kocsis, and Original RNG implementators"),
    ("Readline", "Thies C. Arntzen"),
    ("Reflection", "Marcus Boerger, Timm Friebe, George Schlossnagle, Andrei Zmievski, Johannes Schlueter"),
    ("Sessions", "Sascha Schumann, Andrei Zmievski"),
    ("Shared Memory Operations", "Slava Poliakov, Ilia Alshanetsky"),
    ("SimpleXML", "Sterling Hughes, Marcus Boerger, Rob Richards"),
    ("SNMP", "Rasmus Lerdorf, Harrie Hazewinkel, Mike Jackson, Steven Lawrance, Johann Hanne, Boris Lytochkin"),
    ("SOAP", "Brad Lafountain, Shane Caraveo, Dmitry Stogov"),
    ("Sockets", "Chris Vandomelen, Sterling Hughes, Daniel Beulshausen, Jason Greene"),
    ("Sodium", "Frank Denis"),
    ("SPL", "Marcus Boerger, Etienne Kneuss"),
    ("SQLite 3.x driver for PDO", "Wez Furlong"),
    ("SQLite3", "Scott MacVicar, Ilia Alshanetsky, Brad Dewar"),
    ("System V Message based IPC", "Wez Furlong"),
    ("System V Semaphores", "Tom May"),
    ("System V Shared Memory", "Christian Cartus"),
    ("tidy", "John Coggeshall, Ilia Alshanetsky"),
    ("tokenizer", "Andrei Zmievski, Johannes Schlueter"),
    ("uri", "Máté Kocsis, Tim Düsterhus, Ignace Nyamagana Butera, Arnaud Le Blanc, Dennis Snell, Nora Dossche, Nicolas Grekas"),
    ("XML", "Stig Bakken, Thies C. Arntzen, Sterling Hughes"),
    ("XMLReader", "Rob Richards"),
    ("XMLWriter", "Rob Richards, Pierre-Alain Joye"),
    ("XSL", "Christian Stocker, Rob Richards"),
    ("Zip", "Pierre-Alain Joye, Remi Collet"),
    ("Zlib", "Rasmus Lerdorf, Stefan Roehrich, Zeev Suraski, Jade Nicoletti, Michael Wallner"),
];

/// php_print_credits.
pub(crate) fn print_credits(o: &mut Info, flag: i64) {
    if o.1 && flag & CREDITS_FULLPAGE != 0 {
        crate::ext::standard::info::print_htmlhead(o);
    }
    o.print(if o.1 {
        "<h1>PHP Credits</h1>\n"
    } else {
        "PHP Credits\n"
    });
    if flag & CREDITS_GROUP != 0 {
        o.table_start();
        o.header(&["PHP Group"]);
        o.row(&["Thies C. Arntzen, Stig Bakken, Shane Caraveo, Andi Gutmans, Rasmus Lerdorf, Sam Ruby, Sascha Schumann, Zeev Suraski, Jim Winstead, Andrei Zmievski"]);
        o.table_end();
    }
    if flag & CREDITS_GENERAL != 0 {
        o.table_start();
        o.header(&[if o.1 {
            "Language Design &amp; Concept"
        } else {
            "Language Design & Concept"
        }]);
        o.row(&["Andi Gutmans, Rasmus Lerdorf, Zeev Suraski, Marcus Boerger"]);
        o.table_end();
        o.table_start();
        o.colspan_header(2, "PHP Authors");
        o.header(&["Contribution", "Authors"]);
        for (a, b) in [
            ("Zend Scripting Language Engine", "Andi Gutmans, Zeev Suraski, Stanislav Malyshev, Marcus Boerger, Dmitry Stogov, Xinchen Hui, Nikita Popov"),
            ("Extension Module API", "Andi Gutmans, Zeev Suraski, Andrei Zmievski"),
            ("UNIX Build and Modularization", "Stig Bakken, Sascha Schumann, Jani Taskinen, Peter Kokot"),
            ("Windows Support", "Shane Caraveo, Zeev Suraski, Wez Furlong, Pierre-Alain Joye, Anatol Belski, Kalle Sommer Nielsen"),
            ("Server API (SAPI) Abstraction Layer", "Andi Gutmans, Shane Caraveo, Zeev Suraski"),
            ("Streams Abstraction Layer", "Wez Furlong, Sara Golemon"),
            ("PHP Data Objects Layer", "Wez Furlong, Marcus Boerger, Sterling Hughes, George Schlossnagle, Ilia Alshanetsky"),
            ("Output Handler", "Zeev Suraski, Thies C. Arntzen, Marcus Boerger, Michael Wallner"),
            ("Consistent 64 bit support", "Anthony Ferrara, Anatol Belski"),
        ] {
            o.row(&[a, b]);
        }
        o.table_end();
    }
    if flag & CREDITS_SAPI != 0 {
        o.table_start();
        o.colspan_header(2, "SAPI Modules");
        o.header(&["Contribution", "Authors"]);
        for (a, b) in SAPI_CREDITS {
            o.row(&[a, b]);
        }
        o.table_end();
    }
    if flag & CREDITS_MODULES != 0 {
        o.table_start();
        o.colspan_header(2, "Module Authors");
        o.header(&["Module", "Authors"]);
        for (a, b) in EXT_CREDITS {
            o.row(&[a, b]);
        }
        o.table_end();
    }
    if flag & CREDITS_DOCS != 0 {
        o.table_start();
        o.colspan_header(2, "PHP Documentation");
        for (a, b) in [
            ("Authors", "Mehdi Achour, Friedhelm Betz, Antony Dovgal, Nuno Lopes, Hannes Magnusson, Philip Olson, Georg Richter, Damien Seguy, Jakub Vrana, Adam Harvey"),
            ("Editor", "Peter Cowburn"),
            ("User Note Maintainers", "Daniel P. Brown, Thiago Henrique Pojda"),
            ("Other Contributors", "Previously active authors, editors and other contributors are listed in the manual."),
        ] {
            o.row(&[a, b]);
        }
        o.table_end();
    }
    if flag & CREDITS_QA != 0 {
        o.table_start();
        o.header(&["PHP Quality Assurance Team"]);
        o.row(&["Ilia Alshanetsky, Joerg Behrens, Antony Dovgal, Stefan Esser, Moriyoshi Koizumi, Magnus Maatta, Sebastian Nohn, Derick Rethans, Melvyn Sopacua, Pierre-Alain Joye, Dmitry Stogov, Felipe Pena, David Soria Parra, Stanislav Malyshev, Julien Pauli, Stephen Zarkos, Anatol Belski, Remi Collet, Ferenc Kovacs"]);
        o.table_end();
    }
    if flag & CREDITS_WEB != 0 {
        o.table_start();
        o.colspan_header(2, "Websites and Infrastructure team");
        for (a, b) in [
            ("PHP Websites Team", "Rasmus Lerdorf, Hannes Magnusson, Philip Olson, Lukas Kahwe Smith, Pierre-Alain Joye, Kalle Sommer Nielsen, Peter Cowburn, Adam Harvey, Ferenc Kovacs, Levi Morrison"),
            ("Event Maintainers", "Damien Seguy, Daniel P. Brown"),
            ("Network Infrastructure", "Daniel P. Brown"),
            ("Windows Infrastructure", "Alex Schoenmaker"),
        ] {
            o.row(&[a, b]);
        }
        o.table_end();
    }
    if o.1 && flag & CREDITS_FULLPAGE != 0 {
        o.print("</div></body></html>\n");
    }
}

fn fn_phpcredits(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let flag = args
        .first()
        .map_or(CREDITS_ALL, |v| conv::to_int(&deref(v)));
    let mut o = Info::for_sapi();
    print_credits(&mut o, flag as i32 as i64);
    host.write_out(&o.0);
    Ok(Value::Bool(true))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("phpcredits", fn_phpcredits)]);
}
