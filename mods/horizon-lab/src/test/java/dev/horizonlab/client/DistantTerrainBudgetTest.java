package dev.horizonlab.client;

import com.seibel.distanthorizons.api.DhApi;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfig;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfigValue;
import com.seibel.distanthorizons.api.interfaces.config.client.IDhApiMultiThreadingConfig;
import java.lang.reflect.Proxy;

/** Exercise the actual adapter against the upstream API contracts, without a game process. */
public final class DistantTerrainBudgetTest {
    private static Double override;
    private static double userValue = 0.5;

    @SuppressWarnings("unchecked")
    private static <T> T proxy(Class<T> type, java.lang.reflect.InvocationHandler handler) {
        return (T) Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[]{type}, handler);
    }

    private static void check(boolean value, String reason) { if (!value) throw new AssertionError(reason); }

    public static void main(String[] args) {
        IDhApiConfigValue<?> value = proxy(IDhApiConfigValue.class, (p, method, values) -> switch (method.getName()) {
            case "getTrueValue" -> userValue;
            case "getValue" -> override == null ? userValue : override;
            case "getApiValue" -> override;
            case "getCanBeOverrodeByApi" -> true;
            case "setValue" -> { override = (Double) values[0]; yield true; }
            case "clearValue" -> { override = null; yield true; }
            default -> throw new AssertionError("Unexpected API call: " + method);
        });
        var threading = proxy(IDhApiMultiThreadingConfig.class, (p, method, values) -> value);
        DhApi.Delayed.configs = proxy(IDhApiConfig.class, (p, method, values) -> threading);

        DistantTerrainBudget budget = new DistantTerrainBudget();
        budget.observe(1, 0, 16.667);
        budget.observe(3_000_000_001L, 40_000_000, 16.667);
        check(Math.abs(override - 0.325) < 1e-9, "Actual adapter applies the policy through the API");
        budget.suspend();
        check(override == null && userValue == 0.5, "Suspending releases only our temporary override");
        budget.observe(4_000_000_001L, 0, 16.667);
        budget.observe(7_000_000_001L, 40_000_000, 16.667);
        check(Math.abs(override - 0.325) < 1e-9, "Resuming resets stale frame observations and policy state");
        userValue = 0.1;
        budget.observe(8_000_000_001L, 40_000_000, 16.667);
        budget.observe(11_000_000_001L, 40_000_000, 16.667);
        check(override <= 0.1, "A changed user ceiling is honored");
        override = 0.7;
        budget.observe(12_000_000_001L, 40_000_000, 16.667);
        budget.suspend();
        check(override == 0.7 && budget.status().startsWith("yielded"), "Never clear another mod's changed override");
        budget = new DistantTerrainBudget();
        budget.observe(13_000_000_001L, 40_000_000, 16.667);
        check(override == 0.7 && budget.status().startsWith("yielded"), "Never acquire an existing override");
        System.out.println("Distant terrain API adapter tests passed");
    }
}
