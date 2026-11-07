<?php require __DIR__.'/boot.php';
use Illuminate\Mail\Mailable; use Illuminate\Mail\Mailables\{Content, Envelope}; use Illuminate\Notifications\Messages\MailMessage;
class M extends Mailable { public function envelope(): Envelope { return new Envelope(subject: 'Order shipped'); } public function content(): Content { return new Content(markdown: 'mail::message', with: ['slot' => "# Hello\n\nYour order **#42** shipped.\n\n| a | b |\n|---|---|\n| 1 | 2 |"]); } }
section('markdown-mail', fn() => (new M)->render());
section('notification-mail', fn() => (string) (new MailMessage)->greeting('Hi!')->line('Line one & two.')->action('View', 'https://example.com/x?a=1')->line('Thanks')->render());
section('send-log', function () { config(['mail.default'=>'array']); Mail::to('a@b.c')->send(new M); $m = app('mailer')->getSymfonyTransport()->messages()->first(); return [$m->getOriginalMessage()->getSubject(), (string) $m->getOriginalMessage()->getTo()[0]->toString(), strlen($m->toString()) > 100]; });
section('html-text', fn() => \Illuminate\Support\Str::of((new M)->render())->stripTags()->squish()->value());
