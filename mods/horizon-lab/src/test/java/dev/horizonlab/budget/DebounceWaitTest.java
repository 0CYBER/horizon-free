package dev.horizonlab.budget;

public final class DebounceWaitTest {
    public static void main(String[] args) {
        long debounce = 250_000_000L;
        check(DebounceWait.delay(0, 0, debounce, false) == 1_000_000L, "bounded wait for a fresh update");
        check(DebounceWait.delay(249_900_000L, 0, debounce, false) == 100_000L, "remaining deadline limits wait");
        check(DebounceWait.delay(debounce, 0, debounce, false) == 0, "ready update is not delayed");
        check(DebounceWait.delay(debounce + 1, 0, debounce, false) == 0, "overdue update is not delayed");
        check(DebounceWait.delay(1, 0, debounce, true) == 0, "pending preprocessing is not delayed");
        check(DebounceWait.delay(0, 1, debounce, false) == 0, "future timestamp does not create a wait");
        check(DebounceWait.delay(0, 0, 0, false) == 0, "disabled debounce is not delayed");
        long wrapped = Long.MAX_VALUE - 500_000L;
        check(DebounceWait.delay(wrapped + 1_000_000L, wrapped, debounce, false) == 1_000_000L, "monotonic timer wrap");
        long now = 0;
        int waits = 0;
        while (now < debounce) {
            long wait = DebounceWait.delay(now, 0, debounce, false);
            check(wait > 0 && wait <= DebounceWait.MAX_WAIT_NS && now + wait <= debounce, "deadline unchanged");
            now += wait;
            waits++;
        }
        check(now == debounce && waits == 250, "debounce deadline is reached without extending it");
        System.out.println("Debounce wait policy tests passed.");
    }

    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
}
