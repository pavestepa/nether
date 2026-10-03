fn main() {
  let mut s = "Some"
  let mut a =: User {
    name = "Paul"
  };
  a.hey();
  a.set_name(s)
  a.hey();
  s = "Some2"
  a.hey();
  foo(a);

}

struct User {
  name String
}

impl User {
  hey(:&self) {
    println(self.name);
  }
  set_name(:&mut self, name String) {
    self.name = name;
  }
}

fn foo<T Sound>(u: T) {
  u.sound();
}

trait Sound {
  sound() {
    println("...")
  }
}

impl User Sound {
  sound() {
    println("Human")
  }
}

1) при открытии карты показывать только последние 4 цифры (Экран карты)
2) Кнопка "Пополнить" -> перход на страницы перевода Между своими сметами
Счет пополнения тот который указан на карте
3) Кнопка оплатить (Страницы карты) -> переход на экран услуг с id этой карты
4) История операций (Страница карты) -> переход на экран историй операций с id этой карты
