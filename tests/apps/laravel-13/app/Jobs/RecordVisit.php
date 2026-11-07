<?php

namespace App\Jobs;

use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Foundation\Queue\Queueable;
use Illuminate\Support\Facades\Cache;

// Dispatched on the sync queue: serialized, unserialized and handled in the
// same request.
class RecordVisit implements ShouldQueue
{
    use Queueable;

    public function __construct(public string $who, public array $tags = [])
    {
    }

    public function handle(): void
    {
        Cache::put('rhp-job', ['who' => $this->who, 'tags' => $this->tags, 'attempts' => $this->attempts()]);
    }
}
