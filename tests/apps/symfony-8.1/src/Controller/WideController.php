<?php
namespace App\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\BinaryFileResponse;
use Symfony\Component\HttpFoundation\Cookie;
use Symfony\Component\HttpFoundation\JsonResponse;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\HttpFoundation\Session\SessionInterface;
use Symfony\Component\HttpKernel\Exception\AccessDeniedHttpException;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

final class WideController extends AbstractController
{
    #[Route('/w/session', methods: ['GET'])]
    public function sessionAction(SessionInterface $s): JsonResponse
    {
        $n = $s->get('n', 0) + 1; $s->set('n', $n);
        $this->addFlash('info', "visit $n");
        return $this->json(['n' => $n]);
    }

    #[Route('/w/flash', methods: ['GET'])]
    public function flashAction(Request $r): JsonResponse
    {
        return $this->json($r->getSession()->getFlashBag()->all());
    }

    #[Route('/w/redirect', name: 'w_redirect')]
    public function redirectAction(): Response
    {
        return $this->redirectToRoute('hello', ['name' => 'Ré dir', 'q' => [1, 'a b']]);
    }

    #[Route('/w/urls/{id<\d+>}', name: 'w_urls')]
    public function urlsAction(int $id): JsonResponse
    {
        return $this->json([
            'abs' => $this->generateUrl('w_urls', ['id' => $id + 1, 'x' => 'ü&'], UrlGeneratorInterface::ABSOLUTE_URL),
            'net' => $this->generateUrl('hello', ['name' => 'a/b'], UrlGeneratorInterface::NETWORK_PATH),
            'rel' => $this->generateUrl('twig', ['name' => 'x y'], UrlGeneratorInterface::RELATIVE_PATH),
        ]);
    }

    #[Route('/w/cookie')]
    public function cookieAction(Request $r): Response
    {
        $res = new Response('cookies: '.json_encode($r->cookies->all()));
        $res->headers->setCookie(Cookie::create('a', 'b c;d', 2000000000, '/', null, false, true, false, Cookie::SAMESITE_STRICT));
        $res->headers->setCookie(new Cookie('raw', 'v', 0, '/', null, false, false, true));
        $res->headers->clearCookie('gone');
        return $res;
    }

    #[Route('/w/error')]
    public function errorAction(): Response { throw new \RuntimeException('boom <b>'); }

    #[Route('/w/denied')]
    public function deniedAction(): Response { throw new AccessDeniedHttpException('nope'); }

    #[Route('/w/warning')]
    public function warningAction(): Response { $a = []; return new Response('v='.@$a['x'].((string) (1 / 3)).' '.$a['missing']); }

    #[Route('/w/stream')]
    public function streamAction(): StreamedResponse
    {
        return new StreamedResponse(function () { foreach (['a', 'b', 'c'] as $x) { echo $x, "\n"; flush(); } });
    }

    #[Route('/w/file')]
    public function fileAction(): BinaryFileResponse
    {
        return $this->file(__DIR__.'/../../templates/base.html.twig', 'base é.twig');
    }

    #[Route('/w/json-hex')]
    public function jsonHexAction(): JsonResponse
    {
        return $this->json(['s' => "<a href='x'>&\"</a>", 'u' => "\u{2028}é/"]);
    }

    #[Route('/w/etag')]
    public function etagAction(Request $r): Response
    {
        $res = new Response('etag body');
        $res->setEtag('abc'); $res->setPublic(); $res->setMaxAge(60);
        $res->setLastModified(new \DateTimeImmutable('2026-01-01 00:00:00 UTC'));
        $res->isNotModified($r);
        return $res;
    }

    #[Route('/w/request')]
    public function requestAction(Request $r): JsonResponse
    {
        return $this->json([
            'uri' => $r->getUri(), 'path' => $r->getPathInfo(), 'base' => $r->getBaseUrl(), 'host' => $r->getHttpHost(),
            'scheme' => $r->getScheme(), 'secure' => $r->isSecure(), 'ip' => $r->getClientIp(), 'ajax' => $r->isXmlHttpRequest(),
            'lang' => $r->getPreferredLanguage(['en', 'fr', 'de']), 'langs' => $r->getLanguages(), 'formats' => $r->getAcceptableContentTypes(),
            'format' => $r->getPreferredFormat(), 'int' => $r->query->getInt('i'), 'bool' => $r->query->getBoolean('b'),
            'alpha' => $r->query->getAlpha('s'), 'digits' => $r->query->getDigits('s'), 'enum' => $r->query->getString('s'),
            'qs' => $r->getQueryString(), 'charsets' => $r->getCharsets(), 'etags' => $r->getETags(),
            'f' => 1.0, 'g' => 0.1 + 0.2, 'big' => PHP_INT_MAX, 'neg0' => -0.0, 'u' => "\u{1F600}/</script>",
        ], 200, [], ['json_encode_options' => JsonResponse::DEFAULT_ENCODING_OPTIONS | JSON_PRETTY_PRINT]);
    }

    #[Route('/w/upload', methods: ['POST'])]
    public function uploadAction(Request $r): JsonResponse
    {
        $out = [];
        foreach ($r->files->all() as $k => $f) { $out[$k] = [$f->getClientOriginalName(), $f->getSize(), $f->getMimeType(), $f->guessExtension(), $f->isValid(), md5_file($f->getPathname())]; }
        return $this->json(['files' => $out, 'post' => $r->request->all()]);
    }

    #[Route('/w/twig')]
    public function twigAction(): Response
    {
        return $this->render(($_GET['t'] ?? 'wide').'.html.twig', ['items' => ['b' => 2, 'a' => 1, 'c' => 3.5], 'when' => new \DateTimeImmutable('2026-03-04 05:06:07 UTC'), 'html' => '<i>x</i>', 'n' => 1234567.891]);
    }

    #[Route('/w/put', methods: ['PUT', 'PATCH', 'DELETE'])]
    public function putAction(Request $r): JsonResponse
    {
        return $this->json(['m' => $r->getMethod(), 'body' => $r->getContent(), 'payload' => $r->getPayload()->all()]);
    }
}
