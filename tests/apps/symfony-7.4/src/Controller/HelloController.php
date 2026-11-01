<?php

namespace App\Controller;

use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\JsonResponse;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * The fixture routes the rhp request harness exercises. Keep the output
 * deterministic: no clocks, random values or absolute paths in responses.
 */
final class HelloController extends AbstractController
{
    #[Route('/', name: 'home', methods: ['GET'])]
    public function home(): Response
    {
        return new Response('<html><body><h1>Home</h1></body></html>');
    }

    #[Route('/hello/{name}', name: 'hello', methods: ['GET'], defaults: ['name' => 'World'])]
    public function hello(string $name): Response
    {
        return new Response('<html><body><h1>Hello '.htmlspecialchars($name).'</h1></body></html>');
    }

    #[Route('/json', name: 'json', methods: ['GET'])]
    public function jsonAction(Request $request): JsonResponse
    {
        return $this->json(['ok' => true, 'q' => $request->query->all(), 'n' => [1, 2.5, null, 'é']]);
    }

    #[Route('/twig/{name}', name: 'twig', methods: ['GET'], defaults: ['name' => 'Twig'])]
    public function twig(string $name): Response
    {
        return $this->render('hello/index.html.twig', ['name' => $name, 'items' => ['a', 'b', 'c']]);
    }

    #[Route('/submit', name: 'submit', methods: ['POST'])]
    public function submit(Request $request): JsonResponse
    {
        return new JsonResponse([
            'method' => $request->getMethod(),
            'post' => $request->request->all(),
            'ct' => $request->headers->get('Content-Type'),
            'body' => $request->getContent(),
        ], 201);
    }
}
