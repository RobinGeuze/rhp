//! Time per request through the request layer with and without the
//! compiled-script cache, the way an embedding server runs requests: one
//! worker thread, a fresh engine per request.
//!
//! The app is generated: a PSR-4 autoloader, `classes` entity classes and
//! as many services (an interface, a trait, constructor injection), a
//! container, a controller and a PHP template. Each request autoloads every
//! class, so it includes `2 * classes + 6` files.
//!
//! ```sh
//! cargo run --release -p rhp-sapi --example opcache_bench -- [requests] [classes]
//! ```

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Sapi, Script};

#[derive(Clone, Default)]
struct Body(Rc<RefCell<Vec<u8>>>);

impl Sapi for Body {
    fn name(&self) -> &str {
        "bench"
    }

    fn ub_write(&mut self, data: &[u8]) {
        self.0.borrow_mut().extend_from_slice(data);
    }
}

fn write(path: &Path, src: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, src).unwrap();
}

/// Generate the app under the temp dir; returns its front controller.
fn generate(classes: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rhp-opcache-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let src = root.join("src");
    write(
        &root.join("vendor/autoload.php"),
        r#"<?php
spl_autoload_register(function (string $class): void {
    $file = dirname(__DIR__) . '/src/' . str_replace(['App\\', '\\'], ['', '/'], $class) . '.php';
    if (is_file($file)) {
        require $file;
    }
});
"#,
    );
    write(
        &src.join("Contracts/Service.php"),
        "<?php\nnamespace App\\Contracts;\n\ninterface Service\n{\n    public function describe(): array;\n}\n",
    );
    write(
        &src.join("Support/Labels.php"),
        r#"<?php
namespace App\Support;

trait Labels
{
    private static int $made = 0;

    protected function label(string $kind, int $id): string
    {
        self::$made++;
        return sprintf('%s-%04d', strtolower($kind), $id);
    }
}
"#,
    );
    for i in 0..classes {
        write(
            &src.join(format!("Model/Entity{i}.php")),
            &format!(
                r#"<?php
namespace App\Model;

final class Entity{i}
{{
    public const KIND = 'Entity';

    public function __construct(
        public readonly int $id,
        private array $tags = ['a', 'b', 'c'],
    ) {{
    }}

    public function tags(): array
    {{
        return array_map(fn (string $t): string => strtoupper($t) . $this->id, $this->tags);
    }}

    public function score(): int
    {{
        $sum = 0;
        foreach ($this->tags as $k => $t) {{
            $sum += ($k + 1) * ord($t);
        }}
        return $sum % 97;
    }}
}}
"#
            ),
        );
        write(
            &src.join(format!("Service/Service{i}.php")),
            &format!(
                r#"<?php
namespace App\Service;

use App\Contracts\Service;
use App\Model\Entity{i};
use App\Support\Labels;

class Service{i} implements Service
{{
    use Labels;

    private Entity{i} $entity;

    public function __construct()
    {{
        $this->entity = new Entity{i}({i});
    }}

    public function describe(): array
    {{
        return [
            'label' => $this->label(Entity{i}::KIND, $this->entity->id),
            'tags' => implode(',', $this->entity->tags()),
            'score' => $this->entity->score(),
        ];
    }}
}}
"#
            ),
        );
    }
    write(
        &src.join("Container.php"),
        r#"<?php
namespace App;

final class Container
{
    private array $instances = [];

    public function get(string $id): object
    {
        return $this->instances[$id] ??= new $id();
    }
}
"#,
    );
    write(
        &src.join("Controller/Home.php"),
        &format!(
            r#"<?php
namespace App\Controller;

use App\Container;

final class Home
{{
    public function __construct(private Container $container)
    {{
    }}

    public function __invoke(): string
    {{
        $rows = [];
        for ($i = 0; $i < {classes}; $i++) {{
            $rows[] = $this->container->get("App\\Service\\Service$i")->describe();
        }}
        ob_start();
        require dirname(__DIR__, 2) . '/templates/home.php';
        return ob_get_clean();
    }}
}}
"#
        ),
    );
    write(
        &root.join("templates/home.php"),
        r#"<!doctype html>
<title>Services</title>
<table>
<?php foreach ($rows as $row): ?>
  <tr><td><?= htmlspecialchars($row['label']) ?></td><td><?= $row['tags'] ?></td><td><?= $row['score'] ?></td></tr>
<?php endforeach ?>
</table>
"#,
    );
    let front = root.join("public/index.php");
    write(
        &front,
        r#"<?php
require dirname(__DIR__) . '/vendor/autoload.php';

echo (new App\Controller\Home(new App\Container()))();
"#,
    );
    // Old enough for opcache.file_update_protection.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    for f in walk(&root) {
        let _ = std::fs::File::options()
            .write(true)
            .open(&f)
            .and_then(|f| f.set_modified(old));
    }
    front
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

fn request(front: &Path, opcache: &str) -> Vec<u8> {
    let body = Body::default();
    let outcome = run_request(
        Box::new(body.clone()),
        Request {
            script: Script::File(front.to_string_lossy().into_owned()),
            ini: vec![
                ("opcache.enable".into(), opcache.into()),
                ("display_errors".into(), "1".into()),
            ],
            args: Vec::new(),
            info: Some(RequestInfo {
                method: Some(b"GET".to_vec()),
                response_code: 200,
                ..Default::default()
            }),
            php_binary: None,
        },
    );
    assert_eq!(outcome, Outcome::Exit(0));
    let out = body.0.borrow().clone();
    out
}

fn main() {
    let arg = |i: usize, d: usize| {
        std::env::args()
            .nth(i)
            .and_then(|a| a.parse().ok())
            .unwrap_or(d)
    };
    let (requests, classes) = (arg(1, 200), arg(2, 150));
    let front = generate(classes);
    println!(
        "{requests} requests, {} files included per request",
        2 * classes + 6
    );
    let mut bodies = Vec::new();
    for (label, opcache) in [("opcache.enable=0", "0"), ("opcache.enable=1", "1")] {
        let front = front.clone();
        // A worker thread of its own: the cache is per thread.
        let (per_request, body) = std::thread::spawn(move || {
            let body = request(&front, opcache);
            for _ in 0..5 {
                request(&front, opcache);
            }
            let start = Instant::now();
            for _ in 0..requests {
                request(&front, opcache);
            }
            (start.elapsed() / requests as u32, body)
        })
        .join()
        .unwrap();
        println!(
            "{label}: {:>8.3} ms/request, {:>7.1} requests/s",
            per_request.as_secs_f64() * 1e3,
            1.0 / per_request.as_secs_f64()
        );
        bodies.push(body);
    }
    assert_eq!(bodies[0], bodies[1], "the cache changed the output");
    assert!(
        bodies[0].starts_with(b"<!doctype html>"),
        "{}",
        String::from_utf8_lossy(&bodies[0])
    );
    let _ = std::fs::remove_dir_all(front.parent().unwrap().parent().unwrap());
}
