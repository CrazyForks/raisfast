export class Events {
  private readonly baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl;
  }

  subscribe(filter?: string): EventSource {
    const url = filter
      ? `${this.baseUrl}/events?filter=${encodeURIComponent(filter)}`
      : `${this.baseUrl}/events`;
    return new EventSource(url);
  }
}
