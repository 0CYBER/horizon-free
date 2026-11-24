"""`item predicates`: `/execute if items` sub-predicates (`~`) and exact component equality (`=`)."""

GROUP = 'item predicates'


def cases(cmd, setup):
    """Covers Cluster C6 (#20490): vanilla 26.3 DataComponentPredicates (`~`), exact component
    equality (`=`) on decoded values, prototype defaults, unsigned-byte fireworks flight_duration,
    flattened SNBT custom_data, MobEffectInstance show_icon/hidden_effect, and text component forms."""
    ops = []

    def give(item):
        ops.extend(setup(f'/item replace entity @s weapon.mainhand with {item}'))

    def check(predicate, expect):
        ops.append(cmd(
            f'/execute if items entity @s weapon.mainhand {predicate}',
            expect_result=expect if expect else None,
        ))

    # Plain stone: prototype defaults, custom_data~{} empty default, count presence & bounds.
    give('minecraft:stone 3')
    check('*[minecraft:custom_data~{}]', 3)
    check('*[minecraft:custom_data~{a:1b}]', 0)
    check('*[minecraft:enchantments~[]]', 3)
    check('*[minecraft:stored_enchantments~[]]', 0)
    check('*[minecraft:attribute_modifiers~{modifiers:{size:0}}]', 3)
    check('*[minecraft:max_stack_size~{}]', 3)
    check('*[count]', 3)
    check('*[!count]', 0)
    check('*[count=3]', 3)
    check('*[count~{min:2,max:4}]', 3)

    # Flattened SNBT custom_data string patch with `~` and `=`.
    give('minecraft:stone[minecraft:custom_data="{a:1b,b:2}"]')
    check('*[minecraft:custom_data~{a:1b}]', 1)
    check('*[minecraft:custom_data~"{a:1b}"]', 1)
    check('*[minecraft:custom_data~{a:1}]', 0)
    check('*[minecraft:custom_data={a:1b,b:2}]', 1)

    # Fireworks: prototype flight_duration=1 and unsigned byte flight_duration=200.
    give('minecraft:firework_rocket')
    check('*[minecraft:fireworks~{flight_duration:1,explosions:{size:0}}]', 1)
    check('*[minecraft:fireworks={flight_duration:1,explosions:[]}]', 1)
    give('minecraft:firework_rocket[minecraft:fireworks={flight_duration:200}]')
    check('*[minecraft:fireworks~{flight_duration:200}]', 1)
    check('*[minecraft:fireworks={flight_duration:200,explosions:[]}]', 1)
    check('*[minecraft:fireworks~{flight_duration:1}]', 0)

    # Damage sub-predicate: damage & durability bounds, and !damage removal.
    give('minecraft:diamond_sword[minecraft:damage=100,minecraft:max_damage=250]')
    check('*[minecraft:damage~{damage:100,durability:150}]', 1)
    check('*[minecraft:damage~{durability:{min:200}}]', 0)
    give('minecraft:diamond_sword[!minecraft:damage]')
    check('*[minecraft:damage~{}]', 0)

    # Potion contents: 26.3 `{potions, effects}`, show_icon default, hidden_effect ignored in `=`.
    give('minecraft:potion[minecraft:potion_contents="minecraft:swiftness"]')
    check('*[minecraft:potion_contents~{potions:"minecraft:swiftness",effects:{contains:[{"minecraft:speed":{amplifier:0}}]}}]', 1)
    check('*[minecraft:potion_contents={potion:"swiftness",custom_effects:[]}]', 1)
    give('minecraft:potion[minecraft:potion_contents={custom_effects:[{id:"minecraft:speed",duration:200,show_particles:false,hidden_effect:{amplifier:1b,duration:100}}]}]')
    check('*[minecraft:potion_contents={custom_effects:[{id:"speed",duration:200,show_particles:false,show_icon:false}]}]', 1)
    check('*[minecraft:potion_contents={custom_effects:[{id:"speed",duration:200,show_particles:false,show_icon:true}]}]', 0)

    # Consumable unknown fields ignored, defaults equal.
    give('minecraft:stone[minecraft:consumable={}]')
    check('*[minecraft:consumable={consume_seconds:1.6f,animation:"eat",unknown:1}]', 1)

    # Written book & text component forms (`["Page 1"]` == `"Page 1"` == `{text:"Page 1"}`).
    give('minecraft:written_book[minecraft:written_book_content={title:"Tales",author:"Alex",pages:[["Page 1"]]}]')
    check('*[minecraft:written_book_content~{author:"Alex",title:"Tales",generation:0,resolved:false,pages:{contains:["Page 1"]}}]', 1)
    check('*[minecraft:written_book_content={title:"Tales",author:"Alex",pages:["Page 1"]}]', 1)
    give('minecraft:stone[minecraft:custom_name=["Page 1"],minecraft:item_name=["Blade"],minecraft:lore=[["Line 1"]]]')
    check('*[minecraft:custom_name="Page 1",minecraft:item_name={text:"Blade"},minecraft:lore=["Line 1"]]', 1)

    # Container and bundle nested ItemPredicates.
    give('minecraft:shulker_box[minecraft:container=[{slot:0,item:{id:"minecraft:stone",count:3}}]]')
    check('*[minecraft:container~{items:{size:1,contains:[{items:"minecraft:stone",count:3}]}}]', 1)
    give('minecraft:bundle[minecraft:bundle_contents=[{id:"minecraft:stone",count:2}]]')
    check('*[minecraft:bundle_contents~{items:{count:[{test:{items:"minecraft:stone"},count:1}]}}]', 1)

    return ops + setup('/clear @s')
